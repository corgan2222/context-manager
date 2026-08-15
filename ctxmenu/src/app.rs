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
use crate::program::identity::{NameResolver, Presence};
use crate::registry::backup::{self, BackupManifest};
use crate::registry::create::{self, Fault, NewChild, NewEntry, Problem};
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
        /// How many watched file types the plan reaches beyond the one on
        /// screen. `None` when nothing in it sits on level 1 or 2 of the
        /// resolution chain, which is the only case where the question arises.
        breadth: Option<usize>,
    },
    Running,
    Done(Report),
    Error(String),
    /// Something worked. Its own variant because announcing a finished backup
    /// in a red window titled "Error" is its own small betrayal.
    Note(String),
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
        /// Set when an entry that already exists is being *looked at*: the
        /// registry path it came from.
        ///
        /// The form is then filled in and locked. Writing back is a separate
        /// decision — it would have to know which values changed, keep a
        /// backup first and survive a key that vanished in between — so this
        /// shows and does not touch (2026-08-15).
        existing: Option<String>,
    },
    /// Who made this and which build it is.
    About,
    /// An entry was written; the registry path it landed in.
    ///
    /// Its own variant rather than a `Note`, because it carries a question:
    /// `SHChangeNotify` is enough for a static verb, but a COM handler is a DLL
    /// Explorer loaded long ago and only a restart unloads it. Which of the two
    /// this was is not something the person at the screen can tell, so the
    /// offer is made every time and taken up when it is wanted.
    Created(String),
}

/// Which column the table is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    /// The order the rows were collected in, which carries meaning of its
    /// own: in the file type tab the entries belonging to the chosen
    /// extension come first and the ones that apply to every file follow.
    /// Sorting by name would shuffle those back together.
    Natural,
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
/// Rows are what the selection holds, children included. That was not always
/// so: the selection used to be a set of entry indices, which a child has no
/// way to be, and the comment here justified it by claiming a child's path
/// could not be expressed as a `RegTarget`. It can — `…\shell\<parent>\shell\
/// <child>` names one entry and passes every check — so the limitation was
/// never anything but the shape of the set.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

    /// Does this row contain `other` as a submenu child, at any depth?
    ///
    /// Used to drop a child from a plan when its parent is in the same
    /// selection. Deleting the parent removes the whole subtree, so the
    /// child's own step would then run against a key that is already gone and
    /// report a failure the user did not cause.
    fn is_ancestor_of(&self, other: &Row) -> bool {
        self.entry == other.entry
            && self.path.len() < other.path.len()
            && other.path.starts_with(&self.path)
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

/// The author's mark, as raw RGBA rather than as the PNG it came from.
///
/// `egui_extras` runs here without its `image` feature — the icon extraction
/// builds its own `ColorImage` from GDI pixels, so nothing in this binary can
/// decode a PNG, and adding a decoder for one picture would be a strange
/// trade. The conversion happened once, alongside the download; what is left is
/// 72 KB that need no code at all.
///
/// Stored as a mask: every pixel is white with the original brightness folded
/// into its alpha, so tinting with the current text colour lands the logo in
/// the right shade in both themes. The original is light-on-transparent and
/// would be nearly invisible on a light background.
const LOGO_RGBA: &[u8] = include_bytes!("../assets/logo.rgba");
const LOGO_SIZE: [usize; 2] = [144, 128];

const REPO_URL: &str = "https://github.com/corgan2222/context-manager";
const AUTHOR_URL: &str = "https://github.com/corgan2222";
const AUTHOR_NAME: &str = "Stefan Knaak";

/// A web address as a menu entry, with the doubled `&` the help text warns
/// about. Written out here rather than in `i18n` because a command line is not
/// a translation.
const URL_EXAMPLE: &str = r#"explorer "https://www.google.com/search?q=ctxmenu&&hl=de""#;

/// The Feather glyphs this window draws, looked up once at startup.
///
/// `try_icon` searches a generated table of some three hundred names. Doing that
/// per button per frame is exactly the work ToDo 4.3 keeps out of the frame
/// path, and resolving here has a second benefit: a name a later version of the
/// pack drops turns into a visible blank the moment the window opens, rather
/// than staying unnoticed until somebody looks at that one button.
///
/// Named `Glyphs` and not `Icons` because `App::icons` is already the cache of
/// bitmaps pulled out of executables — these are characters in a font.
#[derive(Debug, Clone, Copy)]
struct Glyphs {
    select_all: char,
    select_none: char,
    visible: char,
    hidden: char,
    always: char,
    shift_only: char,
    free: char,
    blocked: char,
    top: char,
    bottom: char,
    no_position: char,
    delete: char,
    rescan: char,
    new: char,
    backup: char,
    inspect: char,
    copy: char,
}

impl Glyphs {
    /// The names, in one place, so the test below can walk the same list.
    const NAMES: [&'static str; 17] = [
        "check-circle",
        "circle",
        "eye",
        "eye-off",
        "menu",
        "chevrons-up",
        "shield",
        "shield-off",
        "arrow-up-circle",
        "arrow-down-circle",
        "code",
        "trash-2",
        "refresh-cw",
        "plus",
        "save",
        "info",
        "copy",
    ];

    fn load() -> Self {
        let mut glyphs = Self::NAMES.iter().map(|name| feather(name));
        let mut next = || glyphs.next().unwrap_or(' ');
        Glyphs {
            select_all: next(),
            select_none: next(),
            visible: next(),
            hidden: next(),
            always: next(),
            shift_only: next(),
            free: next(),
            blocked: next(),
            top: next(),
            bottom: next(),
            no_position: next(),
            delete: next(),
            rescan: next(),
            new: next(),
            backup: next(),
            inspect: next(),
            copy: next(),
        }
    }
}

/// One Feather glyph by name, or a space when the pack has no such icon.
///
/// A space rather than a panic or a replacement character: a missing icon is a
/// cosmetic problem, and a tool that refuses to start over one would be worse
/// than a button with a gap in front of its label.
fn feather(name: &str) -> char {
    iconflow::try_icon(
        iconflow::Pack::Feather,
        name,
        iconflow::Style::Regular,
        iconflow::Size::Regular,
    )
    .ok()
    .and_then(|icon| char::from_u32(icon.codepoint))
    .unwrap_or(' ')
}

/// A command line to read, select or copy with one click.
///
/// Selectable text alone was not enough: an example is there to be used, and
/// dragging across a line of quotes and percent signs to catch every character
/// is a worse way to spend a click than pressing a button beside it.
fn copyable_command(ui: &mut Ui, glyphs: Glyphs, tr: &'static Strings, command: &str) {
    ui.horizontal(|ui| {
        if ui
            .small_button(glyphs.copy.to_string())
            .on_hover_text(tr.tip_copy)
            .clicked()
        {
            ui.ctx().copy_text(command.to_owned());
        }
        ui.add(
            egui::Label::new(egui::RichText::new(command).monospace())
                .selectable(true)
                .wrap(),
        );
    });
}

/// An icon in front of its label, the shape every button in the bar takes.
fn labelled(icon: char, text: &str) -> String {
    format!("{icon}  {text}")
}

/// Whether the selected rows agree on one property, and on what.
///
/// A segmented switch can only light one segment when every selected row is in
/// it. `Mixed` is not an error: picking one hidden and one visible entry is a
/// normal thing to do, and it simply means no segment is the current one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Agreement<T> {
    /// Nothing selected, or nothing the scan still knows about.
    Empty,
    Same(T),
    Mixed,
}

fn agreement<T: PartialEq>(values: impl IntoIterator<Item = T>) -> Agreement<T> {
    let mut values = values.into_iter();
    let Some(first) = values.next() else {
        return Agreement::Empty;
    };
    match values.all(|other| other == first) {
        true => Agreement::Same(first),
        false => Agreement::Mixed,
    }
}

/// What the action bar needs to know about the selection, in one pass.
///
/// The bar asks the same rows six questions, and answering one means walking
/// each row's submenu path through `resolve`. Doing that once per frame beats
/// doing it once per switch.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectionState {
    /// Rows the user picked, whether or not the scan still knows them.
    count: usize,
    /// Of those, the ones the scan could still resolve. Lower than `count` for
    /// a frame after a rescan replaced the entries behind the rows.
    resolved: usize,
    /// Rows naming a key this program may address at all — the `RegTarget`
    /// check that `plan_for_selection` applies (everything outside `\Classes`,
    /// the CommandStore above all, drops out here).
    changeable: usize,
    /// How many are marked read-only. A number, not a veto: an elevated run
    /// can write what an ordinary one cannot, which is why
    /// `plan_for_selection` deliberately keeps such rows in the plan.
    read_only: usize,
    /// How many carry a CLSID — the one thing blocking needs.
    blockable: usize,
    hidden: Agreement<bool>,
    extended: Agreement<bool>,
    /// Over the rows that *have* a CLSID only: the switch acts on those, so it
    /// is their state it should be showing.
    blocked: Agreement<bool>,
    position: Agreement<Option<String>>,
}

/// Everything this entry can actually be told to do, in menu order.
///
/// "Can" is meant strictly: only the direction that would change something, and
/// only mechanisms this kind of entry has. A hidden entry is offered Show and
/// not Hide; a static verb has no CLSID and is not offered blocking at all;
/// a COM handler gets no position, because the scanner does not even read one
/// for a `shellex` key and setting it would be invisible.
///
/// This is what the bar's greyed-out switches say in the negative, said once
/// more in the affirmative — and the two must not disagree, which is why both
/// go through `clsid_of`.
fn actions_for(entry: &ContextEntry, alone: bool) -> Vec<Action> {
    let mut out = Vec::new();

    out.push(match entry.hidden {
        true => Action::Show,
        false => Action::Hide,
    });
    out.push(match entry.extended {
        true => Action::AlwaysShow,
        false => Action::ShiftOnly,
    });

    if clsid_of(entry).is_some() {
        let blocked = matches!(entry.kind, EntryKind::ShellEx { blocked: true, .. });
        out.push(match blocked {
            true => Action::Unblock,
            false => Action::Block,
        });
    }

    // Position only for a single entry. Twenty rows all sent to the top of the
    // menu are twenty rows in alphabetical order again, one block higher — the
    // action means something for one entry and nothing for a group, so it is
    // not offered for a group.
    if alone && matches!(entry.kind, EntryKind::Verb { .. }) {
        for value in [Some("Top"), Some("Bottom"), None] {
            if entry.position.as_deref() != value {
                out.push(Action::SetPosition(value.map(str::to_string)));
            }
        }
    }

    out.push(Action::Delete);
    out
}

/// The same sentence the switch for this action carries in the bar.
///
/// One explanation per mechanism, reached from both places. A menu line that
/// explained itself differently from the button doing the same thing would be
/// two answers to one question.
fn action_tip(action: &Action, tr: &'static Strings) -> &'static str {
    match action {
        Action::Hide | Action::Show => tr.tip_group_visibility,
        Action::ShiftOnly | Action::AlwaysShow => tr.tip_group_shift,
        Action::Block | Action::Unblock => tr.tip_group_systemwide,
        Action::SetPosition(_) => tr.tip_position,
        Action::Delete => tr.tip_delete,
    }
}

/// An action as a menu line: what it does, not what it is called internally.
fn menu_label(action: &Action, tr: &'static Strings) -> String {
    match action {
        // `action_label` says "Position" for all three, which is fine as a
        // dialog title and useless as a menu line.
        Action::SetPosition(value) => {
            let where_to = match value.as_deref() {
                Some("Top") => tr.pos_top,
                Some("Bottom") => tr.pos_bottom,
                _ => tr.pos_default,
            };
            format!("{}: {where_to}", tr.act_position)
        }
        other => action_label(other, tr).to_string(),
    }
}

/// The icon a menu line carries, matching the switch that does the same thing.
fn menu_glyph(action: &Action, glyphs: &Glyphs) -> char {
    match action {
        Action::Hide => glyphs.hidden,
        Action::Show => glyphs.visible,
        Action::ShiftOnly => glyphs.shift_only,
        Action::AlwaysShow => glyphs.always,
        Action::Block => glyphs.blocked,
        Action::Unblock => glyphs.free,
        Action::SetPosition(value) => match value.as_deref() {
            Some("Top") => glyphs.top,
            Some("Bottom") => glyphs.bottom,
            _ => glyphs.no_position,
        },
        Action::Delete => glyphs.delete,
    }
}

/// What a click on `row` does to the selection.
///
/// The three ways Explorer reads a click, and the reason this is a function of
/// its own rather than a branch inside the table closure: it is the one piece
/// of the table that can be checked without a window.
///
/// - plain: this row alone, and the anchor moves here
/// - Ctrl: toggle this row, and the anchor moves here
/// - Shift: everything from the anchor to here, the anchor staying put; with
///   Ctrl held as well the range is added instead of replacing
fn apply_click(
    selected: &mut rustc_hash::FxHashSet<Row>,
    anchor: &mut Option<Row>,
    visible: &[Row],
    row: &Row,
    ctrl: bool,
    shift: bool,
) {
    let span = shift
        .then(|| {
            let from = anchor
                .as_ref()
                .and_then(|start| visible.iter().position(|candidate| candidate == start))?;
            let to = visible.iter().position(|candidate| candidate == row)?;
            Some(match from <= to {
                true => (from, to),
                false => (to, from),
            })
        })
        .flatten();

    if let Some((first, last)) = span {
        if !ctrl {
            selected.clear();
        }
        for row in &visible[first..=last] {
            selected.insert(row.clone());
        }
        // Deliberately not moved: after Shift-clicking row 20, a second
        // Shift-click on row 5 has to select 5..20, not 5..5.
        return;
    }

    if ctrl {
        if !selected.remove(row) {
            selected.insert(row.clone());
        }
    } else {
        selected.clear();
        selected.insert(row.clone());
    }
    *anchor = Some(row.clone());
}

/// Sums up the selection for the action bar.
fn selection_state(
    scan: Option<&ScanResult>,
    selected: &rustc_hash::FxHashSet<Row>,
) -> SelectionState {
    let mut state = SelectionState {
        count: selected.len(),
        resolved: 0,
        changeable: 0,
        read_only: 0,
        blockable: 0,
        hidden: Agreement::Empty,
        extended: Agreement::Empty,
        blocked: Agreement::Empty,
        position: Agreement::Empty,
    };

    let Some(scan) = scan else {
        return state;
    };

    let entries: Vec<&ContextEntry> = selected
        .iter()
        .filter_map(|row| resolve(scan, row))
        .collect();

    state.resolved = entries.len();
    state.read_only = entries.iter().filter(|entry| entry.read_only).count();
    state.changeable = entries
        .iter()
        .filter(|entry| crate::registry::paths::RegTarget::parse(&entry.registry_path).is_ok())
        .count();
    state.blockable = entries
        .iter()
        .filter(|entry| clsid_of(entry).is_some())
        .count();

    state.hidden = agreement(entries.iter().map(|entry| entry.hidden));
    state.extended = agreement(entries.iter().map(|entry| entry.extended));
    state.position = agreement(entries.iter().map(|entry| entry.position.clone()));
    state.blocked = agreement(entries.iter().filter_map(|entry| match &entry.kind {
        EntryKind::ShellEx { clsid, blocked, .. } if !clsid.is_empty() => Some(*blocked),
        _ => None,
    }));

    state
}

/// The CLSID of a COM handler, if this entry is one and names it.
///
/// The same condition `plan_for_selection` uses to decide whether a row can be
/// blocked at all — shared so the greyed-out switch and the plan cannot
/// disagree about what is blockable.
fn clsid_of(entry: &ContextEntry) -> Option<&str> {
    match &entry.kind {
        EntryKind::ShellEx { clsid, .. } if !clsid.is_empty() => Some(clsid),
        _ => None,
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
    /// What is typed in the "add an extension" field, before it is added.
    ext_draft: String,
    /// Walk *every* registered extension on the next scan, not the curated
    /// list plus the user's own.
    ///
    /// Deliberately not persisted: it is thirteen times the work on this
    /// machine, and a setting that silently makes every future start slow is
    /// not a setting anyone would connect to the button they once pressed.
    scan_every_type: bool,
    /// Index into `groups` for the program tab.
    selected_group: Option<usize>,
    /// Built once after each scan; never in the frame path.
    groups: Vec<ProgramGroup>,
    /// Indices into `scan.entries`. Multi-select, because the whole point of
    /// the program view is acting on twenty entries at once.
    selected: rustc_hash::FxHashSet<Row>,
    /// The row whose details are shown — the last one clicked.
    focused: Option<Row>,
    /// Where a Shift-click measures from.
    ///
    /// Not the same as `focused`, and Explorer keeps them apart for a reason:
    /// after Shift-clicking row 20 the focus is on 20, but a second Shift-click
    /// on row 5 must still select 5..20 rather than 5..5. A plain or Ctrl click
    /// moves the anchor, Shift never does.
    anchor: Option<Row>,
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
    /// Receives the result of a full backup.
    ///
    /// Its own channel and its own thread: a full backup is around forty
    /// `reg.exe` calls, and the frame path cannot wait for that.
    full_backup_rx: Option<Receiver<Result<String, String>>>,

    /// The tool box (`favourites.json`), read on entering the tab and after
    /// every change — never per frame.
    favourites: Vec<Favourite>,
    favourite_error: Option<String>,
    /// Index into `favourites` the keyboard is on.
    ///
    /// Separate from `focused`, which belongs to the scanned table: this tab
    /// shows its own list, and the arrow keys used to move a table nobody
    /// could see while the favourites stayed where they were.
    favourite_focus: Option<usize>,
    /// Set when the keyboard moved the focus, so the row can be scrolled into
    /// view exactly once instead of fighting the mouse wheel every frame.
    favourite_scroll: bool,

    icons: IconCache,
    /// The Feather characters the bar draws, resolved at startup.
    glyphs: Glyphs,
    /// The author's mark. Uploaded to the GPU on first draw, not at startup:
    /// the window opens before anyone looks at the corner it sits in.
    logo: Option<egui::TextureHandle>,
    tr: &'static Strings,
    settings: Settings,

    /// Kept so the title bar can follow later theme switches.
    hwnd: Option<HWND>,
    /// Last dark-mode state pushed to DWM, so the call happens on change only.
    titlebar_dark: Option<bool>,
    /// Language the window title currently carries.
    title_language: Option<Language>,
    titlebar_supported: bool,

    frame_times: FrameTimes,
    bench: Option<Bench>,
    theme_reported: bool,
    /// Running only in the probe mode, and only for as long as it takes.
    theme_probe: Option<ThemeProbe>,
    /// Still owing the window a move to the leftmost screen.
    place_left: bool,
    /// Milliseconds from process creation to the first frame that actually
    /// showed rows — the milestone 12 target of under two seconds.
    ///
    /// Measured from the process creation time rather than from `main`, so the
    /// loader, the static CRT and the window creation are all inside the
    /// number instead of hiding in front of it.
    first_list_ms: Option<f64>,
}

/// Is this category read for every file, whatever its type?
///
/// Levels 1 and 2 of the resolution chain — `*` and `AllFilesystemObjects` —
/// are, which is what makes an action on one of them wider than it looks from
/// inside a single file type.
fn applies_to_every_file_type(category: &Category) -> bool {
    matches!(
        category,
        Category::AllFiles | Category::AllFilesystemObjects
    )
}

/// The count behind the breadth warning, without needing a window.
///
/// `None` when the plan touches nothing on level 1 or 2, and when no file
/// types were scanned at all — a number nobody measured is worse than no
/// sentence. Registry paths are matched case-insensitively, as Windows does.
fn breadth_of_plan(plan: &Plan, entries: &[ContextEntry], file_type_count: usize) -> Option<usize> {
    if file_type_count == 0 {
        return None;
    }

    let touched: rustc_hash::FxHashSet<String> = plan
        .operations
        .iter()
        .map(|operation| operation.target.full_path().to_lowercase())
        .collect();

    entries
        .iter()
        .any(|entry| {
            applies_to_every_file_type(&entry.category)
                && touched.contains(&entry.registry_path.to_lowercase())
        })
        .then_some(file_type_count)
}

/// What the favourites list was asked to do, by mouse or by keyboard.
///
/// One type for both, so a key and the button next to it cannot drift apart:
/// every action is written once and reached from two places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FavouriteAction {
    Edit(usize),
    Place(usize),
    Remove(usize),
    /// `true` moves the entry towards the top of the list.
    Shift(usize, bool),
}

/// Where a list cursor goes on an arrow key, `Home` or `End`.
///
/// Pure, so the rules can be checked without a window. Clamps at both ends
/// rather than wrapping, and lands on the first row when nothing was selected
/// yet — the same behaviour the scanned table has had since milestone 4, since
/// two lists in one window that answer the same key differently is worse than
/// either rule on its own.
fn next_cursor(current: Option<usize>, count: usize, movement: Movement) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match (movement, current) {
        // Home and End say where to go outright; they do not care where the
        // cursor was, or whether there was one.
        (Movement::First, _) => 0,
        (Movement::Last, _) => count - 1,
        (_, None) => 0,
        (Movement::Down, Some(index)) => (index + 1).min(count - 1),
        (Movement::Up, Some(index)) => index.saturating_sub(1),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Movement {
    Down,
    Up,
    First,
    Last,
}

/// Drives the window at full speed for a fixed number of frames, scrolling as
/// it goes, then reports and closes.
///
/// Exists because the milestone 4 target — 60 fps at 2.000 rows — is otherwise
/// only checkable by looking at the window and believing it. Scrolling is part
/// of it: a virtualized table is cheap precisely because it rebuilds only the
/// visible rows, and that rebuild is what has to stay cheap.
/// How many frames the probe lets pass before it flips the setting.
///
/// The first frames carry window creation and the first scan; a reading taken
/// there would record a window still settling rather than a steady state.
const PROBE_SETTLE_FRAMES: usize = 30;

/// How long to wait for the switch to arrive, in frames.
///
/// `WM_SETTINGCHANGE` is broadcast synchronously but every window on the
/// desktop gets it first, so the answer is not in the next frame. At roughly
/// 8 ms per frame this is about two seconds — long past anything observed,
/// short enough that a probe that fails still ends.
const PROBE_WAIT_FRAMES: usize = 240;

/// One reading of everything the theme touches.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ThemeReading {
    /// What Windows told egui, via winit's `ThemeChanged`.
    system: Option<egui::Theme>,
    /// What egui resolved it to, after the preference is applied.
    dark_mode: bool,
    /// What this program last pushed to DWM for the title bar.
    titlebar: Option<bool>,
}

impl ThemeReading {
    fn take(ctx: &egui::Context, ui: &Ui, titlebar: Option<bool>) -> Self {
        Self {
            system: ctx.system_theme(),
            dark_mode: ui.visuals().dark_mode,
            titlebar,
        }
    }
}

impl std::fmt::Display for ThemeReading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "system={:?} dark_mode={} titlebar_dark={:?}",
            self.system, self.dark_mode, self.titlebar
        )
    }
}

/// Drives the theme probe: settle, flip, watch, restore.
struct ThemeProbe {
    stage: ProbeStage,
    frames: usize,
}

enum ProbeStage {
    /// Letting the window reach a steady state before anything is measured.
    Settling { left: usize },
    /// The setting is flipped and the guard is holding the way back.
    Waiting {
        before: ThemeReading,
        left: usize,
        /// Restores the system setting when the probe is dropped. Boxed only
        /// to keep the enum's variants from differing wildly in size, and
        /// never read: holding it *is* what it does, and dropping the probe is
        /// what puts the desktop back.
        #[allow(dead_code)]
        guard: Box<theme::SystemThemeGuard>,
    },
}

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
    last_focus: Option<Row>,
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
        // `start_ext` preselects an extension in the file type tab. It exists
        // so that tab can be measured at all: without a selection it shows
        // nothing, and its one real fault was invisible from outside.
        start_ext: Option<String>,
        // Runs the runtime theme switch probe instead of waiting for a user to
        // change the setting by hand.
        theme_probe: bool,
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
            sort: (SortBy::Natural, true),
            scroll_to_top: false,
            tab: start_tab,
            selected_category: None,
            selected_ext: start_ext,
            ext_draft: String::new(),
            scan_every_type: false,
            selected_group: None,
            groups: Vec::new(),
            selected: rustc_hash::FxHashSet::default(),
            focused: None,
            anchor: None,
            search: start_search,
            dialog: None,
            action_rx: None,
            scan_rx: None,
            scanning: false,
            progress: (0, 0),
            progress_label: String::new(),
            backups: Vec::new(),
            backup_error: None,
            full_backup_rx: None,
            favourites: Vec::new(),
            favourite_error: None,
            favourite_focus: None,
            favourite_scroll: false,
            icons: IconCache::new(&cc.egui_ctx),
            glyphs: Glyphs::load(),
            logo: None,
            tr,
            settings,
            hwnd,
            titlebar_dark: None,
            title_language: None,
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
            // Every run that started itself goes to the left screen: the
            // main one is the user's desk.
            place_left: theme_probe || bench_frames.is_some(),
            theme_probe: theme_probe.then(|| ThemeProbe {
                stage: ProbeStage::Settling {
                    left: PROBE_SETTLE_FRAMES,
                },
                frames: 0,
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
        // Decided here, built in the worker: enumerating every registered
        // extension means reading two large keys, and the frame path has no
        // business doing that (ToDo 4.3).
        let every_type = self.scan_every_type;
        let custom = self.settings.custom_extensions.clone();

        std::thread::Builder::new()
            .name("registry-scan".into())
            .spawn(move || {
                let options = if every_type {
                    ScanOptions::with_every_installed_file_type()
                } else {
                    ScanOptions::with_file_types(&custom)
                };
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

                    // Levels 1 and 2 apply to every file, so they are part of
                    // what a right-click on this type really offers (ToDo
                    // 10.4) — but they are also identical for every type, and
                    // for `.jpg` they are 39 rows against 19. Off by default
                    // since 2026-08-15, and one checkbox away.
                    if self.settings.include_generic_entries {
                        rows.extend(scan.entries.iter().enumerate().filter_map(|(i, e)| {
                            matches!(
                                e.category,
                                Category::AllFiles | Category::AllFilesystemObjects
                            )
                            .then_some(i)
                        }));
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

            Tab::Backups | Tab::Favourites => Vec::new(),
        };

        let needle = self.search.trim().to_lowercase();
        // Deduplicated while keeping the order. One entry legitimately belongs
        // to several file types — a `SystemFileAssociations\image` handler is
        // shared by every image extension — so a candidate list gathered
        // across types holds the same index several times. Left in, the same
        // row appears repeatedly, the count is wrong, a backup would export
        // one key several times, and the keyboard cursor cannot get past the
        // duplicate at all: the position lookup always finds the first copy,
        // so the arrow key walks back to it every time.
        let mut seen = rustc_hash::FxHashSet::default();
        let mut candidates: Vec<usize> = candidates
            .into_iter()
            .filter(|index| seen.insert(*index))
            .filter(|index| needle.is_empty() || matches_search(&scan.entries[*index], &needle))
            .collect();

        // Sorted here, once per change, not per frame. Children keep their
        // place under the parent they belong to: a cascading menu that sorted
        // itself apart would stop being a menu.
        let (column, ascending) = self.sort;
        let tr = self.tr;
        if column != SortBy::Natural {
            candidates.sort_by(|a, b| {
                let (a, b) = (&scan.entries[*a], &scan.entries[*b]);
                let ordering = sort_key(a, column, tr).cmp(&sort_key(b, column, tr));
                if ascending {
                    ordering
                } else {
                    ordering.reverse()
                }
            });
        }

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
            && let Some(first) = self.visible_rows.first()
        {
            self.focused = Some(first.clone());
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

        // And not while a text field has the keyboard. Home, End and the
        // arrows are editing keys in the search box, and consuming them there
        // would break typing to fix the very list this moves through.
        if ctx.memory(|memory| memory.focused()).is_some() {
            return None;
        }

        // Ctrl+A, the way every file manager reads it. It was missing entirely:
        // the button existed, the shortcut everybody tries first did not.
        // `COMMAND` rather than `CTRL` is egui's platform-correct spelling, and
        // the guard above keeps it out of the search box, where Ctrl+A has to
        // go on meaning "select this text".
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
            self.selected = self.visible_rows.iter().cloned().collect();
            // A range that a later Shift-click measures from has to start
            // somewhere; the top of the list is the only honest answer.
            self.anchor = self.visible_rows.first().cloned();
        }

        // Every visible row, not only the top-level ones: a submenu child is
        // an entry of its own and the arrows now reach it.
        let stops: Vec<usize> = (0..self.visible_rows.len()).collect();
        if stops.is_empty() {
            return None;
        }

        let current = self
            .focused
            .as_ref()
            .and_then(|focused| self.visible_rows.iter().position(|row| row == focused))
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

        let row = self.visible_rows[stops[next]].clone();
        self.focused = Some(row.clone());
        if !extend {
            self.selected.clear();
            // Arrowing to a row without Shift is the same statement as clicking
            // it: this is where a later Shift-click measures from.
            self.anchor = Some(row.clone());
        }
        self.selected.insert(row);

        // The row to scroll to, in table coordinates.
        Some(stops[next])
    }

    /// The keyboard for the favourites list.
    ///
    /// Arrows, `Home` and `End` move the cursor; `Enter` places the favourite
    /// — the one thing the whole tab exists for — and `Delete` removes it,
    /// exactly as the two buttons on the row do. Returns the action so it can
    /// be applied where the mouse's own actions are, rather than reaching into
    /// the list from two directions.
    fn handle_favourite_keys(&mut self, ctx: &egui::Context) -> Option<FavouriteAction> {
        // Same two guards as the table: a dialog owns the keyboard while it is
        // up, and so does a text field that has the focus.
        if self.dialog.is_some() || ctx.memory(|memory| memory.focused()).is_some() {
            return None;
        }

        let count = self.favourites.len();
        if count == 0 {
            self.favourite_focus = None;
            return None;
        }
        // A removal can leave the cursor past the end of the list.
        if let Some(index) = self.favourite_focus {
            self.favourite_focus = Some(index.min(count - 1));
        }

        let (down, up, home, end, enter, delete) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Home),
                i.consume_key(egui::Modifiers::NONE, egui::Key::End),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
            )
        });

        let movement = match (down, up, home, end) {
            (true, ..) => Some(Movement::Down),
            (_, true, ..) => Some(Movement::Up),
            (_, _, true, _) => Some(Movement::First),
            (_, _, _, true) => Some(Movement::Last),
            _ => None,
        };

        if let Some(movement) = movement {
            self.favourite_focus = next_cursor(self.favourite_focus, count, movement);
            self.favourite_scroll = true;
            return None;
        }

        let index = self.favourite_focus?;
        match (enter, delete) {
            (true, _) => Some(FavouriteAction::Place(index)),
            (_, true) => Some(FavouriteAction::Remove(index)),
            _ => None,
        }
    }

    /// Carries out one action from the favourites list.
    ///
    /// By index rather than by id because that is what the keyboard has; the
    /// id is resolved here, once, and a stale index simply does nothing rather
    /// than acting on whichever entry moved into that slot.
    fn apply_favourite_action(&mut self, action: FavouriteAction) {
        let index = match action {
            FavouriteAction::Edit(index)
            | FavouriteAction::Place(index)
            | FavouriteAction::Remove(index)
            | FavouriteAction::Shift(index, _) => index,
        };
        let Some(favourite) = self.favourites.get(index).cloned() else {
            return;
        };

        match action {
            FavouriteAction::Edit(_) => {
                self.dialog = Some(Dialog::Favourite {
                    draft: Box::new(favourite),
                    fresh: false,
                });
            }
            FavouriteAction::Place(_) => {
                // Same rule as the editor: start where the user last was. The
                // favourites tab has no tree of its own, so this is whatever
                // they were looking at before coming here — which is exactly
                // the thing they wanted this tool for.
                let category = self.category_for_new();
                self.dialog = Some(Dialog::Place {
                    favourite: Box::new(favourite),
                    ext: match &category {
                        Category::ExtAssoc(ext) | Category::ExtDirect(ext) => ext.clone(),
                        _ => String::new(),
                    },
                    perceived: match &category {
                        Category::PerceivedType(kind) => kind.clone(),
                        _ => "image".into(),
                    },
                    category,
                });
            }
            FavouriteAction::Remove(_) => {
                self.after_favourite_change(favourites::remove(&favourite.id));
            }
            FavouriteAction::Shift(_, up) => {
                self.after_favourite_change(favourites::shift(&favourite.id, up));
                // The cursor follows the entry it was on, or the list would
                // reorder itself out from under the keyboard.
                if self.favourite_focus == Some(index) {
                    let moved = if up {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(self.favourites.len().saturating_sub(1))
                    };
                    self.favourite_focus = Some(moved);
                    self.favourite_scroll = true;
                }
            }
        }
    }

    /// Keeps the window title in the language on screen.
    ///
    /// The title is set once when the window is created, before the settings
    /// are read, so it started life German whatever the setting said — the one
    /// caption that switching the language did not reach. Sent on change only:
    /// a viewport command every frame would be a message to the window manager
    /// sixty times a second for nothing.
    fn sync_title(&mut self, ctx: &egui::Context) {
        if self.title_language == Some(self.settings.language) {
            return;
        }
        self.title_language = Some(self.settings.language);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(window_title(self.tr)));
    }

    /// Moves the window to the leftmost screen, once, on automatic runs.
    ///
    /// Only for runs nobody started by hand — a probe, a benchmark, a smoke
    /// test. A window that a person opened belongs wherever that person wants
    /// it, and dragging it away would be its own kind of rude.
    ///
    /// Not done through `ViewportBuilder`, because the handle does not exist
    /// before the window does; the first frame is the earliest moment this can
    /// happen at all.
    fn place_window_once(&mut self) {
        if !self.place_left {
            return;
        }
        let Some(hwnd) = self.hwnd else { return };
        self.place_left = false;

        match theme::place_on_left_screen(hwnd) {
            Some(placed) => crate::errln!(
                "window_placed: x={} y={} {}x{} physical, leftmost screen",
                placed.x,
                placed.y,
                placed.width,
                placed.height
            ),
            None => crate::errln!("window_placed: FAILED, the window is wherever it opened"),
        }
        crate::console::flush();
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
            bench.last_focus = self.focused.clone();
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
            // What "New" would open on, from wherever the run ended up. A
            // preselection that quietly falls back to the default looks
            // exactly like one that works.
            crate::errln!(
                "bench: new_entry_category={}",
                self.category_for_new().slug()
            );
            crate::console::flush();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Advances the theme probe, if one is running.
    ///
    /// Answers the one question the handover kept open: does
    /// `ThemePreference::System` follow a theme switch made **while the window
    /// is up**? Only the startup case was ever proven, and proving this one by
    /// hand needs somebody to sit in front of the Settings app at the right
    /// moment — which is why it stayed open through twelve milestones.
    ///
    /// The probe flips the setting itself, waits, and puts it back. Both
    /// halves are reported separately, because they fail separately: egui
    /// repaints its own widgets, while the title bar is drawn by DWM and only
    /// changes if this program asks it to.
    fn drive_theme_probe(&mut self, ctx: &egui::Context, ui: &Ui) {
        let Some(probe) = &mut self.theme_probe else {
            return;
        };
        ctx.request_repaint();
        probe.frames += 1;

        match &mut probe.stage {
            ProbeStage::Settling { left } => {
                *left -= 1;
                if *left > 0 {
                    return;
                }
                let before = ThemeReading::take(ctx, ui, self.titlebar_dark);
                match theme::SystemThemeGuard::flip() {
                    Ok((guard, now_light)) => {
                        crate::errln!(
                            "theme_probe: before {before}; flipped system to {}",
                            match now_light {
                                true => "light",
                                false => "dark",
                            }
                        );
                        crate::console::flush();
                        probe.stage = ProbeStage::Waiting {
                            before,
                            left: PROBE_WAIT_FRAMES,
                            guard: Box::new(guard),
                        };
                    }
                    Err(error) => {
                        crate::errln!("theme_probe: could not write the setting: {error:#}");
                        crate::console::flush();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }

            ProbeStage::Waiting { before, left, .. } => {
                let now = ThemeReading::take(ctx, ui, self.titlebar_dark);
                let reacted = now.system != before.system;
                *left -= 1;

                if !reacted && *left > 0 {
                    return;
                }

                let before = *before;
                let frames = probe.frames;
                // Dropping the probe restores the setting through the guard,
                // before this process can be closed or killed.
                self.theme_probe = None;

                crate::errln!("theme_probe: after {now}");
                crate::errln!(
                    "theme_probe: system_theme_followed={} egui_repainted_dark_mode={} titlebar_followed={} frames={frames}",
                    reacted,
                    now.dark_mode != before.dark_mode,
                    now.titlebar != before.titlebar,
                );
                crate::errln!(
                    "theme_probe: verdict={}",
                    match (reacted, now.dark_mode != before.dark_mode) {
                        (true, true) => "ThemePreference::System follows a runtime switch",
                        (true, false) =>
                            "system theme arrived but the visuals did not change — check the preference",
                        _ =>
                            "no reaction: the RegNotifyChangeKeyValue fallback from ToDo 9.1 is due",
                    }
                );
                crate::console::flush();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Clears the selection and takes the view back to the top.
    ///
    /// Called whenever the list is about to show a different set — another
    /// category, extension or program.
    fn clear_selection(&mut self) {
        self.selected.clear();
        self.focused = None;
        self.anchor = None;
        self.scroll_to_top = true;
    }

    /// Builds a plan from the current selection.
    ///
    /// Two kinds of row are dropped: one whose path is not a `RegTarget` — a
    /// submenu container, say — and, for block and unblock, one without a
    /// CLSID, since blocking is a COM-handler mechanism and a static verb has
    /// no equivalent.
    ///
    /// A **read-only** row is deliberately *not* dropped. An earlier version of
    /// this comment claimed it was, on the grounds that a doomed step wastes a
    /// backup and a prompt. It costs a line in the report instead, and that is
    /// the better trade: the user selected the row, and "it failed, here is
    /// why" beats a row that silently never happened. `read_only` is measured,
    /// not guessed, and an elevated run can write what an ordinary one cannot.
    fn plan_for_selection(&self, action: Action) -> Plan {
        let Some(scan) = &self.scan else {
            return Plan::new("leer", Vec::new());
        };

        let mut operations = Vec::new();
        for row in &self.selected {
            // A child whose parent is also selected gets no step of its own:
            // the parent's delete takes the whole subtree, and a second step
            // against the same key would report a failure nobody caused.
            if self.selected.iter().any(|other| other.is_ancestor_of(row)) {
                continue;
            }
            let Some(entry) = resolve(scan, row) else {
                continue;
            };
            let Ok(target) = crate::registry::paths::RegTarget::parse(&entry.registry_path) else {
                continue;
            };

            let clsid = clsid_of(entry).map(str::to_string);
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
            // "Nothing selected" would be a lie when something is: the rows
            // are simply not expressible as a target, which on this machine
            // means the CommandStore. Saying which of the two it is saves the
            // user from clicking again to see whether it worked this time.
            let message = match self.selected.is_empty() {
                true => self.tr.msg_no_selection,
                false => self.tr.msg_nothing_changeable,
            };
            self.dialog = Some(Dialog::Error(message.to_string()));
            return;
        }
        // Probing writability touches the registry, so it happens here on the
        // click and not while drawing the dialog.
        let needs_elevation = plan.needs_elevation();
        let breadth = self.breadth_of(&plan);
        self.dialog = Some(Dialog::Confirm {
            plan,
            needs_elevation,
            breadth,
        });
    }

    /// How many watched file types this plan would also reach.
    ///
    /// `None` unless something in it sits on level 1 or 2 of the resolution
    /// chain. That is the case worth a sentence: the file type tab shows the
    /// entries of `.zip` next to the ones every file gets, they look alike in
    /// the table, and deleting one of the latter while thinking about `.zip`
    /// takes it away from all 98 types at once (ToDo 10.4).
    ///
    /// Read from the plan rather than from the selection, because the plan is
    /// what will actually run — an entry that was selected but dropped on the
    /// way must not produce a warning about something that is not going to
    /// happen.
    fn breadth_of(&self, plan: &Plan) -> Option<usize> {
        let scan = self.scan.as_ref()?;
        breadth_of_plan(plan, &scan.entries, scan.file_types.len())
    }

    /// The category the editor should open on.
    fn category_for_new(&self) -> Category {
        let focused = self
            .focused
            .as_ref()
            .and_then(|row| resolve(self.scan.as_ref()?, row));

        category_for_new_entry(
            focused,
            self.tab,
            self.selected_ext.as_deref(),
            self.selected_category.as_ref(),
        )
    }

    /// Backs up without changing anything.
    ///
    /// The selection, or everything currently listed when nothing is selected.
    /// Until now a backup only ever happened as a by-product of a change,
    /// which meant "let me keep this state before I start poking" had no
    /// button at all.
    fn backup_now(&mut self) {
        let Some(scan) = &self.scan else { return };

        let rows: Vec<Row> = if self.selected.is_empty() {
            self.visible_rows.clone()
        } else {
            self.selected.iter().cloned().collect()
        };

        let paths: Vec<String> = rows
            .iter()
            .filter_map(|row| resolve(scan, row))
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
                // A backup that worked is not an error. This said so in the
                // red error window for as long as the button existed, which
                // made a successful safety net look like a fault.
                self.dialog = Some(Dialog::Note(
                    self.tr.fmt_backup_created.replace("{}", &directory),
                ));
            }
            Err(error) => self.dialog = Some(Dialog::Error(format!("{error:#}"))),
        }
    }

    /// Backs up every place this tool touches, on a worker thread.
    ///
    /// The "look first, decide later" button next to it takes what is on
    /// screen; this one takes the lot, including the branches no scan
    /// returned — the containers, so entries added since the last scan come
    /// along too.
    fn full_backup(&mut self, ctx: &egui::Context) {
        if self.full_backup_rx.is_some() {
            return;
        }

        let (tx, rx) = channel();
        let ctx = ctx.clone();

        std::thread::Builder::new()
            .name("full-backup".into())
            .spawn(move || {
                let paths = crate::registry::paths::full_backup_paths();
                let result = backup::export("gesamt", &paths)
                    .map(|token| token.directory().display().to_string())
                    .map_err(|error| format!("{error:#}"));
                let _ = tx.send(result);
                ctx.request_repaint();
            })
            .expect("backup thread");

        self.full_backup_rx = Some(rx);
    }

    /// Picks up the result of a full backup. Never blocks.
    fn poll_full_backup(&mut self) {
        let Some(rx) = &self.full_backup_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(directory)) => {
                self.full_backup_rx = None;
                self.reload_backups();
                self.dialog = Some(Dialog::Note(
                    self.tr.fmt_backup_created.replace("{}", &directory),
                ));
            }
            Ok(Err(error)) => {
                self.full_backup_rx = None;
                self.dialog = Some(Dialog::Error(error));
            }
            // Same lesson as `poll_scan`: a dead channel has to end the wait,
            // or the button stays disabled for the rest of the session and
            // looks exactly like a crash.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.full_backup_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
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
        self.poll_full_backup();
        self.icons.poll(&ctx);
        self.place_window_once();
        self.sync_titlebar(ui);
        self.sync_title(&ctx);

        if self.filter_dirty {
            self.rebuild_visible();
            self.filter_dirty = false;
        }

        // Before the keyboard is read, not after: the benchmark feeds
        // synthetic key presses into this frame's event queue, and a check
        // that runs first would always measure nothing.
        self.drive_bench(&ctx);
        // After `sync_titlebar` above, so a reading of the title bar reflects
        // this frame rather than the previous one.
        self.drive_theme_probe(&ctx, ui);
        // The keyboard belongs to the list that is on screen. Until now the
        // arrows always moved the scanned table, including on the two tabs
        // that do not show it — so on the favourites tab they moved a
        // selection nobody could see, and left it moved when the user came
        // back.
        let mut favourite_action = None;
        let scroll_to = match std::mem::take(&mut self.scroll_to_top) {
            true => Some(0),
            false => match self.tab {
                Tab::Favourites => {
                    favourite_action = self.handle_favourite_keys(&ctx);
                    None
                }
                Tab::Backups => None,
                Tab::Categories | Tab::FileTypes | Tab::Programs => self.handle_keys(&ctx),
            },
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
                egui::CentralPanel::default()
                    .show(ui, |ui| self.favourite_list(ui, favourite_action));
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
                    .add_enabled(
                        !self.scanning,
                        egui::Button::new(labelled(self.glyphs.rescan, self.tr.btn_rescan)),
                    )
                    .on_hover_text(self.tr.tip_rescan)
                    .on_disabled_hover_text(self.tr.tip_rescan)
                    .clicked()
                {
                    self.start_scan(ctx);
                }

                // These two live up here rather than in the action bar below
                // because neither one acts on the selection: a new entry is
                // created from nothing, and a backup without a selection covers
                // everything visible. Down there they were the two buttons that
                // stayed live while the rest went grey, which is exactly what
                // made the greying look arbitrary.
                //
                // Drawn on every tab, greyed on the two that keep their own
                // lists: dropping a widget shifts the automatic ids of the ones
                // after it, and the search box is one of those.
                let on_entries = !matches!(self.tab, Tab::Backups | Tab::Favourites);
                if ui
                    .add_enabled(
                        on_entries,
                        egui::Button::new(labelled(self.glyphs.new, self.tr.editor_new)),
                    )
                    .on_hover_text(self.tr.tip_editor_new)
                    .on_disabled_hover_text(self.tr.tip_entry_tabs_only)
                    .clicked()
                {
                    self.dialog = Some(Dialog::Editor {
                        entry: Box::new(NewEntry {
                            category: self.category_for_new(),
                            key_name: String::new(),
                            display_name: String::new(),
                            command: String::new(),
                            icon: None,
                            position: None,
                            extended: false,
                            children: Vec::new(),
                        }),
                        recorded: create::recorded().unwrap_or_default(),
                        existing: None,
                    });
                }

                // "Look first, decide later" is a legitimate way to use this
                // program, and until this button existed a backup only ever
                // happened as a side effect of changing something.
                if ui
                    .add_enabled(
                        on_entries,
                        egui::Button::new(labelled(self.glyphs.backup, self.tr.btn_backup_now)),
                    )
                    .on_hover_text(self.tr.tip_backup_now)
                    .on_disabled_hover_text(self.tr.tip_entry_tabs_only)
                    .clicked()
                {
                    self.backup_now();
                }

                let search = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text(self.tr.search_hint)
                            .desired_width(260.0),
                    )
                    .on_hover_text(self.tr.tip_search);
                // Rebuilding on `changed()` instead of every frame is what
                // keeps typing responsive at a few thousand rows (ToDo 11.5).
                if search.changed() {
                    self.filter_dirty = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // First in a right-to-left layout means furthest right.
                    let logo = self.logo_texture(ctx);
                    let tint = ui.visuals().text_color();
                    let height = ui.spacing().interact_size.y;
                    let width = height * LOGO_SIZE[0] as f32 / LOGO_SIZE[1] as f32;
                    // `Button::image`, not `ImageButton`: egui 0.36 folded that
                    // type into `Button` as well, the same way it did with
                    // `SelectableLabel`.
                    if ui
                        .add(
                            egui::Button::image(
                                egui::Image::from_texture(&logo)
                                    .fit_to_exact_size(egui::vec2(width, height))
                                    .tint(tint),
                            )
                            .frame(false),
                        )
                        .on_hover_text(self.tr.tip_about)
                        .clicked()
                    {
                        self.dialog = Some(Dialog::About);
                    }
                    self.settings_controls(ui, ctx);
                });
            });
            ui.add_space(4.0);
        });
    }

    /// The logo, uploaded on first use and kept afterwards.
    fn logo_texture(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        self.logo
            .get_or_insert_with(|| {
                // Unmultiplied: the mask was written with straight alpha, and
                // the premultiplied constructor would darken every soft edge —
                // the same trap the icon cache documents in the other
                // direction, where GDI hands back premultiplied pixels.
                let image = egui::ColorImage::from_rgba_unmultiplied(LOGO_SIZE, LOGO_RGBA);
                ctx.load_texture("logo", image, egui::TextureOptions::LINEAR)
            })
            .clone()
    }

    fn settings_controls(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let mut changed = false;

        let theme = egui::ComboBox::from_id_salt("theme")
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
        theme.response.on_hover_text(self.tr.tip_theme);

        let language_box = egui::ComboBox::from_id_salt("language")
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
        language_box.response.on_hover_text(self.tr.tip_language);

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
    /// The bar between the tabs and the table. Everything in it acts on the
    /// selected rows, and nothing in it acts on anything else.
    ///
    /// That split is the point. Until 2026-08-15 the bar mixed the two: "new
    /// entry" and "back up now" need no selection and were therefore always
    /// live, while the six flag buttons beside them were grey most of the
    /// time. The greying looked arbitrary because the bar was answering two
    /// different questions at once. Those two buttons moved up into the tab
    /// row, and what is left obeys one sentence — pick rows, then use these.
    ///
    /// The six flag buttons themselves became three switches. `Hide` and
    /// `Show` are not two things a user wants, they are two directions on one
    /// axis, and a switch can show which end the selection is at — which no
    /// arrangement of separate buttons can.
    fn action_bar(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        // Both of these tabs act on their own list, not on scanned entries;
        // a bar full of buttons that would apply to nothing is worse than no
        // bar at all.
        if matches!(self.tab, Tab::Backups | Tab::Favourites) {
            return;
        }

        // While a dialog is up its answer is the only thing that matters.
        // These are `egui::Window`s, not modals, so without this the buttons
        // behind them stay live — and pressing one would replace the dialog
        // and throw away the plan waiting inside it.
        let idle = self.dialog.is_none();
        let state = selection_state(self.scan.as_ref(), &self.selected);
        let any = state.count > 0;
        let tr = self.tr;
        let glyphs = self.glyphs;

        // Gathered here and acted on after the panel closes. `propose` swaps
        // the dialog out, and doing that halfway through drawing the bar would
        // leave the rest of it drawn against a state nobody has seen yet.
        let mut wanted = None;
        let mut select_all = false;
        let mut select_none = false;

        egui::Panel::top("actions").show(ui, |ui| {
            ui.add_space(3.0);
            ui.add_enabled_ui(idle, |ui| {
                // Wrapped rather than one fixed row: four groups and a delete
                // button are wider than this window has to be, and the old bar
                // ran off the right edge instead of moving to a second line.
                ui.horizontal_wrapped(|ui| {
                    // No count and no hint here any more: both moved to the
                    // status bar. A line whose text grows and shrinks with the
                    // selection pushed every button beside it sideways, so the
                    // icons never sat still long enough to be aimed at.
                    ui.label(egui::RichText::new(format!("{}:", tr.group_selection)).weak());
                    if ui
                        .button(labelled(glyphs.select_all, tr.btn_select_all))
                        .on_hover_text(tr.tip_select_all)
                        .clicked()
                    {
                        select_all = true;
                    }
                    if ui
                        .add_enabled(
                            any,
                            egui::Button::new(labelled(glyphs.select_none, tr.btn_select_none)),
                        )
                        .on_hover_text(tr.tip_select_none)
                        .on_disabled_hover_text(tr.tip_select_none)
                        .clicked()
                    {
                        select_none = true;
                    }

                    ui.separator();

                    if let Some(action) = switch_groups(ui, tr, &glyphs, &state) {
                        wanted = Some(action);
                    }

                    ui.separator();

                    // Visually set apart: this is the one action a backup cannot
                    // be shrugged off for.
                    let delete = egui::Button::new(
                        egui::RichText::new(labelled(glyphs.delete, tr.btn_delete))
                            .color(ui.visuals().error_fg_color),
                    );
                    if ui
                        .add_enabled(any, delete)
                        .on_hover_text(tr.tip_delete)
                        .on_disabled_hover_text(match any {
                            true => tr.tip_delete,
                            false => tr.tip_needs_selection,
                        })
                        .clicked()
                    {
                        wanted = Some(Action::Delete);
                    }
                    // Next to the one button that reads as dangerous, which is
                    // where the reassurance is worth reading.
                    ui.small(tr.msg_backup_first);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(match elevation::is_elevated() {
                            true => tr.status_elevated,
                            false => tr.status_not_elevated,
                        });
                    });
                });
            });
            ui.add_space(3.0);
        });

        if select_all {
            self.selected = self.visible_rows.iter().cloned().collect();
        }
        if select_none {
            self.clear_selection();
        }
        if let Some(action) = wanted {
            self.propose(action);
        }
        let _ = ctx;
    }

    /// Der Werkzeugkasten.
    ///
    /// Deliberately not a tree and not a table: this list is short by nature —
    /// it holds what one person reaches for often — and its order is the
    /// user's own, so it is shown exactly as saved.
    fn favourite_list(&mut self, ui: &mut Ui, from_keyboard: Option<FavouriteAction>) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading(self.tr.tab_favourites);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(self.tr.fav_new)
                    .on_hover_text(self.tr.tip_fav_new)
                    .clicked()
                {
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
        let mut action = from_keyboard;
        let mut clicked_row: Option<usize> = None;
        let focus = self.favourite_focus;
        let scroll = std::mem::take(&mut self.favourite_scroll);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let count = self.favourites.len();
                for (index, favourite) in self.favourites.iter().enumerate() {
                    let current = focus == Some(index);
                    // The cursor has to be visible or the arrow keys look
                    // broken: the list would move and nothing on screen would
                    // say so.
                    let frame = match current {
                        true => egui::Frame::new()
                            .fill(ui.visuals().selection.bg_fill)
                            .inner_margin(egui::Margin::symmetric(4, 2)),
                        false => egui::Frame::new().inner_margin(egui::Margin::symmetric(4, 2)),
                    };

                    let row = frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.set_min_width(340.0);
                                ui.strong(&favourite.name);
                                ui.small(describe(favourite, self.tr));
                            });

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button(self.tr.fav_remove)
                                        .on_hover_text(self.tr.tip_fav_remove)
                                        .clicked()
                                    {
                                        action = Some(FavouriteAction::Remove(index));
                                    }
                                    if ui
                                        .small_button(self.tr.fav_edit)
                                        .on_hover_text(self.tr.tip_fav_edit)
                                        .clicked()
                                    {
                                        action = Some(FavouriteAction::Edit(index));
                                    }
                                    if ui
                                        .add_enabled(
                                            index + 1 < count,
                                            egui::Button::new("\u{2193}").small(),
                                        )
                                        .on_hover_text(self.tr.tip_fav_down)
                                        .clicked()
                                    {
                                        action = Some(FavouriteAction::Shift(index, false));
                                    }
                                    if ui
                                        .add_enabled(
                                            index > 0,
                                            egui::Button::new("\u{2191}").small(),
                                        )
                                        .on_hover_text(self.tr.tip_fav_up)
                                        .clicked()
                                    {
                                        action = Some(FavouriteAction::Shift(index, true));
                                    }
                                    ui.separator();
                                    // The whole point of the list: putting a
                                    // tool where it can be reached.
                                    if ui
                                        .button(self.tr.fav_place)
                                        .on_hover_text(self.tr.tip_fav_place)
                                        .clicked()
                                    {
                                        action = Some(FavouriteAction::Place(index));
                                    }
                                },
                            );
                        });
                    });

                    // Clicking a row puts the keyboard on it, so the two ways
                    // of moving through the list agree on where "here" is.
                    if row.response.interact(egui::Sense::click()).clicked() {
                        clicked_row = Some(index);
                    }
                    if current && scroll {
                        ui.scroll_to_rect(row.response.rect, None);
                    }
                    ui.separator();
                }
            });

        if let Some(index) = clicked_row {
            self.favourite_focus = Some(index);
        }

        if let Some(action) = action {
            self.apply_favourite_action(action);
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
        .resizable([true, false])
        .default_width(640.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            // Same rule as the entry editor: the fields take what the window
            // gives them, minus the label column and the button behind them.
            // A path is as long as it is, and 440 pixels was a guess.
            let field_width = (ui.available_width() - 150.0).max(240.0);
            let icons = &mut self.icons;

            egui::Grid::new("fav-grid")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label(self.tr.fav_name);
                    ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(field_width));
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
                Tool::Program { path, args } => {
                    program_form(ui, self.tr, path, args, field_width, icons)
                }
                Tool::Web(web) => web_form(ui, self.tr, web, field_width),
            }

            ui.separator();
            let problems = draft.problems();
            for problem in &problems {
                ui.colored_label(ui.visuals().warn_fg_color, fav_fault_text(problem, self.tr));
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
                // Same as the editor: the path when there is one, and
                // otherwise the reason in the list below rather than the raw,
                // bilingual error from `paths`.
                if let Ok(target) = entry.target() {
                    ui.small(format!("\u{2192} {}", target.full_path()));
                }
                ui.small(&entry.command);

                let problems = crate::registry::create::check(&entry);
                let blocked = problems.iter().any(Problem::is_error) || entry.target().is_err();
                for problem in &problems {
                    let colour = match problem {
                        Problem::Error(_) => ui.visuals().error_fg_color,
                        Problem::Warning(_) => ui.visuals().warn_fg_color,
                    };
                    ui.colored_label(colour, fault_text(problem.fault(), self.tr));
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
                breadth,
            } => {
                let mut start = false;
                let mut cancel = false;

                // Not `plan.label`: that one is the internal name, German and
                // fixed, because it also becomes the backup directory and must
                // not change with the interface language. The title says the
                // same thing in the language on screen.
                let title = plan
                    .operations
                    .first()
                    .map(|operation| action_label(&operation.action, self.tr))
                    .unwrap_or(self.tr.detail_title);

                egui::Window::new(title)
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

                        // The reach of the change, before the list of what it
                        // touches: a key path says which type it was found
                        // under, never which types it will be missing from.
                        if let Some(count) = breadth {
                            ui.add_space(4.0);
                            ui.colored_label(
                                ui.visuals().warn_fg_color,
                                self.tr
                                    .fmt_affects_other_types
                                    .replace("{}", &count.to_string()),
                            );
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
                            if ui
                                .button(self.tr.btn_execute)
                                .on_hover_text(self.tr.tip_execute)
                                .clicked()
                            {
                                start = true;
                            }
                            if ui
                                .button(self.tr.btn_cancel)
                                .on_hover_text(self.tr.tip_cancel)
                                .clicked()
                            {
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
                        breadth,
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
                let mut restart = false;

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
                        ui.horizontal(|ui| {
                            if ui.button(self.tr.btn_cancel).clicked() {
                                close = true;
                            }
                            // Next to the sentence that says a restart is
                            // needed, because being told what to do and then
                            // being left to do it by hand is half an answer.
                            if ui
                                .button(self.tr.btn_restart_explorer)
                                .on_hover_text(self.tr.tip_restart_explorer)
                                .clicked()
                            {
                                restart = true;
                            }
                        });
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
                if restart {
                    // Blocking, for up to the two and a half seconds the wait
                    // for the taskbar is capped at. A worker thread would buy
                    // a smoother frame at the price of a second code path for
                    // reporting the failure, and this runs once, on a click.
                    if let Err(error) = elevation::restart_explorer() {
                        self.dialog = Some(Dialog::Error(format!("{error:#}")));
                        keep = false;
                    } else {
                        close = true;
                    }
                }
                if !close && keep {
                    self.dialog = Some(Dialog::Done(report));
                }
                keep = false;
            }

            Dialog::Note(message) => {
                let mut close = false;
                egui::Window::new(self.tr.title_note)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        ui.label(&message);
                        ui.add_space(6.0);
                        if ui
                            .button(self.tr.btn_close)
                            .on_hover_text(self.tr.tip_cancel)
                            .clicked()
                        {
                            close = true;
                        }
                    });
                if !close {
                    self.dialog = Some(Dialog::Note(message));
                }
                keep = false;
            }

            Dialog::Created(path) => {
                let mut close = false;
                let mut restart = false;
                egui::Window::new(self.tr.title_note)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        ui.label(self.tr.fmt_entry_created.replace("{}", &path));
                        ui.add_space(6.0);
                        ui.add(egui::Label::new(self.tr.msg_ask_restart).wrap());
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button(self.tr.btn_restart_explorer)
                                .on_hover_text(self.tr.tip_restart_explorer)
                                .clicked()
                            {
                                restart = true;
                            }
                            if ui.button(self.tr.btn_close).clicked() {
                                close = true;
                            }
                        });
                    });

                if restart {
                    match elevation::restart_explorer() {
                        Ok(()) => close = true,
                        Err(error) => {
                            self.dialog = Some(Dialog::Error(format!("{error:#}")));
                            keep = false;
                        }
                    }
                }
                if !close && keep {
                    self.dialog = Some(Dialog::Created(path));
                }
                keep = false;
            }

            Dialog::About => {
                let mut close = false;
                let mut open_url: Option<&str> = None;
                let logo = self.logo_texture(ui.ctx());
                egui::Window::new(self.tr.about_title)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(4.0);
                            ui.add(
                                egui::Image::from_texture(&logo)
                                    .fit_to_exact_size(egui::vec2(
                                        LOGO_SIZE[0] as f32,
                                        LOGO_SIZE[1] as f32,
                                    ))
                                    .tint(ui.visuals().text_color()),
                            );
                            ui.add_space(10.0);
                            ui.heading(self.tr.app_title);
                            // The same number the title bar and `--version`
                            // carry, from the same constant.
                            ui.label(egui::RichText::new(crate::VERSION).weak());
                            ui.add_space(8.0);
                            ui.label(AUTHOR_NAME);
                            ui.add_space(8.0);
                            // `ui.link` plus this program's own opener, not
                            // `hyperlink_to`: egui hands the address to the
                            // integration, and eframe only carries a native
                            // opener when it is built with the feature for it —
                            // which this one is not, so the links did nothing at
                            // all. `webtool::shell::open` is here anyway, is
                            // used by every favourite, and refuses anything that
                            // is not http, https or file.
                            for (label, url) in [
                                (self.tr.about_repo, REPO_URL),
                                (self.tr.about_profile, AUTHOR_URL),
                            ] {
                                if ui.link(label).on_hover_text(url).clicked() {
                                    open_url = Some(url);
                                }
                            }
                            ui.add_space(10.0);
                            if ui.button(self.tr.btn_close).clicked() {
                                close = true;
                            }
                            ui.add_space(2.0);
                        });
                    });
                if let Some(url) = open_url
                    && let Err(error) = crate::webtool::shell::open(url)
                {
                    // Rare, but silence here would look exactly like the bug
                    // this replaced: a link that does nothing when clicked.
                    self.dialog = Some(Dialog::Error(format!("{error:#}")));
                    close = true;
                }
                if !close {
                    self.dialog = Some(Dialog::About);
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
                existing,
            } => {
                let mut close = false;
                let mut save = false;
                // Looking at an entry that is already in the registry. The
                // fields are filled in and usable — trying values out is half
                // of understanding what an entry does — but nothing is written
                // back yet, and the note at the top says so rather than a grey
                // form implying it.
                let viewing = existing.is_some();
                // Borrowed out here: the closure below already holds `self`
                // for `self.tr`, and the cache needs a mutable borrow to queue
                // an extraction for a reference it has not seen yet.
                let icons = &mut self.icons;

                egui::Window::new(if viewing {
                    self.tr.editor_view_title
                } else {
                    self.tr.editor_title
                })
                .collapsible(false)
                // Wide paths are the normal case — `"C:\Program Files\…" "%1"`
                // does not fit in any width worth defaulting to — so the
                // window can be pulled wider and the fields follow (see
                // `field_width` below). Height stays automatic: it is the sum
                // of the rows, and dragging it would only ever add blank space.
                .resizable([true, false])
                .default_width(620.0)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    ui.set_min_width(560.0);
                    // What a text field may take: everything the window offers
                    // minus the label column, the buttons behind the field and
                    // the frame. Recomputed per frame, so dragging the edge
                    // moves the fields with it.
                    let field_width = (ui.available_width() - 150.0).max(240.0);
                    if let Some(path) = &existing {
                        ui.small(path);
                        ui.colored_label(ui.visuals().warn_fg_color, self.tr.editor_view_note);
                        ui.add_space(4.0);
                    }
                    // Not switched off any more: the form is the same one a
                    // new entry uses, and typing in it is how somebody finds
                    // out what a value does. What it does *not* do is save —
                    // that is one button, and it is not there.
                    {
                        egui::Grid::new("editor-grid")
                            .num_columns(2)
                            .spacing([10.0, 6.0])
                            .show(ui, |ui| {
                                ui.label(self.tr.editor_category);
                                // A file type the entry came in with is offered
                                // as its own choice. Without this the picker
                                // would show "Dateitypen" for `.png` and lose
                                // the preselection at the first click.
                                let chosen = (!Category::BASE.contains(&entry.category))
                                    .then(|| entry.category.clone());

                                ui.horizontal(|ui| {
                                    egui::ComboBox::from_id_salt("editor-category")
                                        .selected_text(category_choice_label(
                                            &entry.category,
                                            self.tr,
                                        ))
                                        .show_ui(ui, |ui| {
                                            if let Some(chosen) = &chosen {
                                                let label = category_choice_label(chosen, self.tr);
                                                ui.selectable_value(
                                                    &mut entry.category,
                                                    chosen.clone(),
                                                    label,
                                                );
                                                ui.separator();
                                            }
                                            for candidate in Category::BASE {
                                                let label = category_label(&candidate, self.tr);
                                                ui.selectable_value(
                                                    &mut entry.category,
                                                    candidate,
                                                    label,
                                                );
                                            }

                                            // The two that take a value of
                                            // their own. Without them the only
                                            // reachable file type was whichever
                                            // one the user happened to come
                                            // from, so "I want this for .png"
                                            // meant going to the file type tab
                                            // first and finding .png there.
                                            ui.separator();
                                            if ui
                                                .selectable_label(
                                                    matches!(entry.category, Category::ExtAssoc(_)),
                                                    self.tr.fav_place_ext,
                                                )
                                                .clicked()
                                            {
                                                entry.category = Category::ExtAssoc(String::new());
                                            }
                                            if ui
                                                .selectable_label(
                                                    matches!(
                                                        entry.category,
                                                        Category::PerceivedType(_)
                                                    ),
                                                    self.tr.fav_place_perceived,
                                                )
                                                .clicked()
                                            {
                                                entry.category =
                                                    Category::PerceivedType("image".into());
                                            }
                                        });

                                    // The value belonging to the choice, right
                                    // next to it rather than in a row of its
                                    // own that would be empty most of the time.
                                    match &entry.category {
                                        Category::ExtAssoc(current) => {
                                            let mut ext = current.clone();
                                            if ui
                                                .add(
                                                    egui::TextEdit::singleline(&mut ext)
                                                        .desired_width(110.0)
                                                        .hint_text(".png"),
                                                )
                                                .changed()
                                            {
                                                entry.category = Category::ExtAssoc(ext);
                                            }
                                        }
                                        Category::PerceivedType(current) => {
                                            let current = current.clone();
                                            for kind in
                                                ["image", "video", "audio", "text", "compressed"]
                                            {
                                                if ui
                                                    .selectable_label(current == kind, kind)
                                                    .clicked()
                                                {
                                                    entry.category =
                                                        Category::PerceivedType(kind.into());
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                });
                                ui.end_row();

                                // Before the name, because it decides what the
                                // rest of the form even asks for: a submenu has
                                // no command line of its own.
                                ui.label(self.tr.editor_kind);
                                ui.horizontal(|ui| {
                                    if ui
                                        .selectable_label(
                                            !entry.is_submenu(),
                                            self.tr.editor_kind_single,
                                        )
                                        .clicked()
                                    {
                                        entry.children.clear();
                                    }
                                    if ui
                                        .selectable_label(
                                            entry.is_submenu(),
                                            self.tr.editor_kind_submenu,
                                        )
                                        .on_hover_text(self.tr.tip_editor_submenu)
                                        .clicked()
                                        && entry.children.is_empty()
                                    {
                                        // A submenu with no children is a menu
                                        // item that opens onto nothing, so the
                                        // mode starts with one empty row rather
                                        // than with an invalid entry.
                                        entry.children.push(empty_child());
                                    }
                                });
                                ui.end_row();

                                ui.label(self.tr.editor_display_name);
                                let before = entry.display_name.clone();
                                ui.add(
                                    egui::TextEdit::singleline(&mut entry.display_name)
                                        .desired_width(field_width),
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
                                        .desired_width(field_width),
                                );
                                ui.end_row();

                                // Hidden rather than greyed out for a submenu:
                                // the value would not be written, and a field
                                // that quietly does nothing is worse than an
                                // absent one.
                                if !entry.is_submenu() {
                                    ui.label(self.tr.editor_command);
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::TextEdit::singleline(&mut entry.command)
                                                .desired_width(field_width - 26.0)
                                                .hint_text(HINT_COMMAND),
                                        );
                                        // Pick the program instead of typing
                                        // its path. What comes back is quoted
                                        // and given `"%1"`, which is what the
                                        // field would have needed anyway.
                                        if folder_button(ui, icons, self.tr.tip_pick_program)
                                            && let Some(path) = crate::filedialog::pick_file(
                                                None,
                                                &crate::filedialog::PROGRAMS,
                                                &entry.command,
                                            )
                                        {
                                            entry.command =
                                                format!("\"{}\" \"%1\"", path.display());
                                        }
                                    });
                                    ui.end_row();
                                } else {
                                    ui.label(self.tr.editor_children);
                                    ui.vertical(|ui| {
                                        children_editor(
                                            ui,
                                            &mut entry.children,
                                            icons,
                                            self.tr,
                                            field_width,
                                        );
                                    });
                                    ui.end_row();
                                }

                                ui.label(self.tr.editor_icon);
                                ui.horizontal(|ui| {
                                    let mut icon = entry.icon.clone().unwrap_or_default();
                                    ui.add(
                                        egui::TextEdit::singleline(&mut icon)
                                            .desired_width(field_width - 52.0)
                                            .hint_text(HINT_ICON),
                                    );
                                    let mut icon = icon.trim().to_string();

                                    // The same picker as for the command, with
                                    // `.ico`, `.exe` and `.dll` offered: an
                                    // icon reference is a file first, and the
                                    // index after the comma is the second step.
                                    if folder_button(ui, icons, self.tr.tip_pick_icon)
                                        && let Some(path) = crate::filedialog::pick_file(
                                            None,
                                            &crate::filedialog::ICONS,
                                            &icon,
                                        )
                                    {
                                        // `,0` because a reference is split at
                                        // its last comma: without an index a
                                        // path containing one would lose
                                        // everything behind it.
                                        icon = format!("{},0", path.display());
                                    }

                                    // What the reference actually resolves to,
                                    // beside the field that names it. The table
                                    // has shown these all along; the form that
                                    // *writes* one showed nothing, so a typo in
                                    // `shell32.dll,-244` was invisible until
                                    // the entry sat in the real menu.
                                    if !icon.is_empty() {
                                        let texture = icons.get(&icon).clone();
                                        ui.add(egui::Image::new(egui::load::SizedTexture::new(
                                            texture.id(),
                                            egui::vec2(16.0, 16.0),
                                        )));
                                    }
                                    entry.icon = (!icon.is_empty()).then_some(icon);
                                });
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
                    }

                    ui.add_space(6.0);
                    // Where it will land, once that is decidable. When it
                    // is not, the reason stands in the list below in one
                    // language — printing the error from `paths` here
                    // instead put a bilingual sentence on screen, usually
                    // directly above the plain reason for it.
                    let target = entry.target();
                    if let Ok(target) = &target
                        && !viewing
                    {
                        ui.small(format!("\u{2192} {}", target.full_path()));
                    }

                    // Live, because a warning after the fact is no use: the %1
                    // trap costs an entry that looks right and does nothing.
                    // Not while looking at something that already exists:
                    // its faults are not this reader's to fix, and half of
                    // them ("key name is missing") would be about fields
                    // that are locked anyway.
                    let problems = if viewing {
                        Vec::new()
                    } else {
                        create::check(&entry)
                    };
                    // `target` too: an unusable category has to disable the
                    // button, or the dialog offers to do something it will
                    // then refuse. Since `check` reports that case itself,
                    // this is now belt and braces rather than the only
                    // guard — and it stays for exactly that reason.
                    let blocked = problems.iter().any(Problem::is_error) || target.is_err();
                    if !problems.is_empty() {
                        ui.add_space(4.0);
                        for problem in &problems {
                            let colour = match problem {
                                Problem::Error(_) => ui.visuals().error_fg_color,
                                Problem::Warning(_) => ui.visuals().warn_fg_color,
                            };
                            ui.colored_label(colour, fault_text(problem.fault(), self.tr));
                        }
                    }

                    // What the placeholders mean. Folded away, because it is
                    // needed once and then never again — and the one that
                    // actually catches people out (`%1` in a background
                    // category) is checked above and said out loud anyway.
                    ui.add_space(4.0);
                    egui::CollapsingHeader::new(self.tr.editor_help)
                        .id_salt("editor-placeholders")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(self.tr.help_placeholders).strong());
                            egui::Grid::new("editor-placeholder-grid")
                                .num_columns(2)
                                .spacing([12.0, 2.0])
                                .show(ui, |ui| {
                                    for (token, meaning) in PLACEHOLDERS {
                                        ui.label(egui::RichText::new(*token).monospace());
                                        ui.label(meaning(self.tr));
                                        ui.end_row();
                                    }
                                });

                            // A command line that works, per shape of category.
                            // The placeholder table says what `%1` and `%V`
                            // stand for; it does not say that picking the wrong
                            // one leaves the command with an empty argument,
                            // and that is the mistake people actually make.
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new(self.tr.help_examples).strong());
                            for (command, meaning) in EXAMPLES {
                                ui.add_space(4.0);
                                ui.add(egui::Label::new(meaning(self.tr)).wrap());
                                copyable_command(ui, self.glyphs, self.tr, command);
                            }

                            ui.add_space(10.0);
                            ui.label(egui::RichText::new(self.tr.help_urls_title).strong());
                            ui.add(egui::Label::new(self.tr.help_urls).wrap());
                            ui.add_space(4.0);
                            // With an `&` in it on purpose: the paragraph above
                            // says it has to be doubled, and a rule is easier to
                            // believe with the example beside it.
                            copyable_command(ui, self.glyphs, self.tr, URL_EXAMPLE);
                        });

                    // What this tool created before — a reminder while
                    // adding something, and noise while reading somebody
                    // else's entry.
                    if !recorded.is_empty() && !viewing {
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
                        if viewing {
                            if ui.button(self.tr.btn_close).clicked() {
                                close = true;
                            }
                        } else {
                            if ui
                                .add_enabled(!blocked, egui::Button::new(self.tr.editor_create))
                                .clicked()
                            {
                                save = true;
                            }
                            if ui.button(self.tr.btn_cancel).clicked() {
                                close = true;
                            }
                        }
                    });
                });

                if save {
                    // Checked again here, immediately before writing. The
                    // button is already disabled while an error stands, so this
                    // is the second line rather than the first — but the form
                    // can be edited after the button was enabled, the category
                    // can be switched to one where `%1` is wrong, and a check
                    // that only ever ran while drawing would miss it.
                    let errors: Vec<String> = create::check(&entry)
                        .iter()
                        .filter(|problem| matches!(problem, Problem::Error(_)))
                        .map(|problem| fault_text(problem.fault(), self.tr))
                        .collect();

                    if !errors.is_empty() {
                        self.dialog = Some(Dialog::Error(errors.join("\n")));
                    } else {
                        match create::create(&entry) {
                            Ok(target) => {
                                // Without this the entry exists but the running
                                // Explorer keeps showing yesterday's menu.
                                elevation::notify_shell();
                                self.start_scan(ctx);
                                // The notification is enough for a static verb
                                // and not for a COM handler, and nobody can
                                // tell which case they are in from the outside.
                                // So the question gets asked rather than the
                                // answer assumed.
                                self.dialog = Some(Dialog::Created(target.full_path()));
                            }
                            Err(error) => {
                                self.dialog = Some(Dialog::Error(format!("{error:#}")));
                            }
                        }
                    }
                } else if !close {
                    self.dialog = Some(Dialog::Editor {
                        entry,
                        recorded,
                        existing,
                    });
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

                    // The selection is reported here rather than above the
                    // table. Up there its text grew and shrank with the number
                    // of selected rows and shoved every button beside it
                    // sideways; an icon that moves cannot be aimed at. Down
                    // here the line is free to change length.
                    if !matches!(self.tab, Tab::Backups | Tab::Favourites) {
                        ui.separator();
                        let state = selection_state(self.scan.as_ref(), &self.selected);
                        ui.label(selection_summary(self.tr, &state));
                    }
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
                        ui.label(self.tr.status_no_dwm);
                    }
                });
            });
        });
    }

    fn category_tree(&mut self, ui: &mut Ui) {
        let mut drop_target: Option<Category> = None;
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

                            // Where a dropped file would land. Noted while
                            // drawing, because that is the only moment this
                            // row's rectangle and the category it stands for
                            // are both at hand. `rect_contains_pointer` rather
                            // than `hovered`: during a drag from outside the
                            // window egui hands out no hover.
                            if ui.rect_contains_pointer(response.rect) {
                                drop_target = Some(category.clone());
                            }

                            if response.clicked() {
                                self.selected_category = Some(category.clone());
                                self.clear_selection();
                                self.filter_dirty = true;
                            }
                        }

                        // Below a separator, and last: these verbs are not on
                        // anybody's menu. Putting them among the categories
                        // that are would suggest they were.
                        let store = Category::CommandStore;
                        let count = self.category_count(&store);
                        ui.separator();
                        let response = ui
                            .add_enabled_ui(count > 0, |ui| {
                                ui.selectable_label(
                                    self.selected_category.as_ref() == Some(&store),
                                    format!("{}  ({count})", category_label(&store, self.tr)),
                                )
                            })
                            .inner
                            .on_hover_text(self.tr.tip_command_store);
                        if response.clicked() {
                            self.selected_category = Some(store);
                            self.clear_selection();
                            self.filter_dirty = true;
                        }

                        ui.take_available_space();
                    });
            });

        self.take_dropped_files(ui.ctx(), drop_target);
    }

    /// Turns files dropped on the window into a filled-in editor form.
    ///
    /// The category comes from whatever the pointer was over, so dragging a
    /// program onto "Desktop-Hintergrund" produces an entry for the desktop
    /// background — with `%V`, because `%1` is empty there. Dropped anywhere
    /// else, the category is the one already selected, which is what the "new
    /// entry" button uses too.
    ///
    /// Nothing is written: the form opens and waits, exactly as if it had been
    /// filled in by hand.
    fn take_dropped_files(&mut self, ctx: &egui::Context, target: Option<Category>) {
        // Cheap per frame: `dropped_files` is empty in every frame but the one
        // where something landed. `path()` is a trait method in egui 0.36, not
        // the `Option<PathBuf>` field it used to be — the integration owns the
        // handle now, and on the web there would be no local path at all.
        //
        // The first file only: the form describes one entry, and twenty forms
        // stacked on top of each other would be a worse answer than one.
        let Some(path) = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .first()
                .map(|file| file.path().to_path_buf())
        }) else {
            return;
        };

        let category = target
            .and_then(|category| creatable_category(&category))
            .unwrap_or_else(|| self.category_for_new());

        self.dialog = Some(Dialog::Editor {
            entry: Box::new(create::from_dropped_file(&path, category)),
            recorded: create::recorded().unwrap_or_default(),
            existing: None,
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
                let generic = &mut self.settings.include_generic_entries;
                if ui
                    .checkbox(generic, self.tr.filter_include_generic)
                    .on_hover_text(self.tr.tip_filter_include_generic)
                    .changed()
                {
                    let _ = self.settings.save();
                    self.filter_dirty = true;
                }

                // Adding a type of one's own, and the full sweep. Both were
                // promised from the start: the curated list is 98 types, this
                // machine has 1928 registered, and `custom_extensions` was
                // saved to disk from milestone 5 on while nothing ever read it.
                let mut add = false;
                let mut sweep = false;
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.ext_draft)
                            .desired_width(72.0)
                            .hint_text(self.tr.ext_hint),
                    );
                    // Enter does what the button does; a one-field form that
                    // insists on the mouse is a form nobody uses twice.
                    let entered =
                        field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    let usable = crate::registry::filetypes::normalize_ext(&self.ext_draft)
                        .is_some_and(|ext| !self.settings.custom_extensions.contains(&ext));

                    add = (ui
                        .add_enabled(usable, egui::Button::new("+"))
                        .on_hover_text(self.tr.tip_ext_add)
                        .on_disabled_hover_text(self.tr.tip_ext_add)
                        .clicked()
                        || entered)
                        && usable;

                    sweep = ui
                        .add_enabled(
                            !self.scan_every_type && !self.scanning,
                            egui::Button::new(self.tr.ext_scan_every),
                        )
                        .on_hover_text(self.tr.tip_ext_scan_every)
                        .on_disabled_hover_text(self.tr.tip_ext_scan_every)
                        .clicked();
                });
                if self.scan_every_type {
                    ui.small(self.tr.ext_every_active);
                }
                if add {
                    // Normalised on the way in, so `.PNG`, `png` and `*.png`
                    // are one type rather than three tree entries pointing at
                    // the same registry keys.
                    if let Some(ext) = crate::registry::filetypes::normalize_ext(&self.ext_draft) {
                        self.settings.custom_extensions.push(ext);
                        let _ = self.settings.save();
                        self.ext_draft.clear();
                        self.start_scan(ui.ctx());
                    }
                }
                if sweep {
                    self.scan_every_type = true;
                    self.start_scan(ui.ctx());
                }

                ui.separator();

                let Some(scan) = &self.scan else {
                    ui.spinner();
                    return;
                };

                let hide_empty = self.settings.hide_empty_types;
                let tr = self.tr;
                let custom = self.settings.custom_extensions.clone();
                let mut clicked: Option<String> = None;
                let mut forget: Option<String> = None;

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
                                    let own = custom.iter().any(|e| e == info.ext());

                                    ui.horizontal(|ui| {
                                        if ui.selectable_label(selected, label).clicked() {
                                            clicked = Some(info.ext().to_string());
                                        }
                                        // Only the user's own types can be
                                        // taken away again; the curated list is
                                        // not the user's to shorten, and a
                                        // dead button on every row would say
                                        // otherwise.
                                        if own
                                            && ui
                                                .small_button("\u{00d7}")
                                                .on_hover_text(tr.tip_ext_remove)
                                                .clicked()
                                        {
                                            forget = Some(info.ext().to_string());
                                        }
                                    });
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
                if let Some(ext) = forget {
                    self.settings.custom_extensions.retain(|e| e != &ext);
                    let _ = self.settings.save();
                    if self.selected_ext.as_deref() == Some(ext.as_str()) {
                        self.selected_ext = None;
                    }
                    self.start_scan(ui.ctx());
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
                        .on_hover_text(self.tr.tip_fav_add_from_program)
                        .on_disabled_hover_text(self.tr.tip_fav_add_from_program)
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
                // Borrowed out of `self` so the row closure can hold the group
                // list and the icon cache at the same time.
                let Self {
                    groups,
                    icons,
                    selected_group,
                    tr,
                    ..
                } = self;

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Three branches, like the file type tree: what is
                        // broken, what the user installed, and what belongs to
                        // Windows. As a badge under every second row this said
                        // the same thing and read as if it belonged to the row
                        // *underneath* it.
                        let branch = |wanted: Branch| -> Vec<usize> {
                            groups
                                .iter()
                                .enumerate()
                                .filter(|(_, g)| Branch::of(g) == wanted)
                                .map(|(index, _)| index)
                                .collect()
                        };

                        for kind in Branch::ALL {
                            let members = branch(kind);
                            // No heading for an empty branch — and above all
                            // none for "gone", which is a finding when it is
                            // there and a question nobody asked when it is not.
                            if members.is_empty() {
                                continue;
                            }

                            let heading = egui::RichText::new(format!(
                                "{}  ({})",
                                kind.label(tr),
                                members.len()
                            ));
                            let heading = if kind == Branch::Gone {
                                heading.color(ui.visuals().error_fg_color)
                            } else {
                                heading
                            };

                            egui::CollapsingHeader::new(heading)
                                .id_salt(kind.salt())
                                // System components are the longest branch and
                                // the one nobody goes looking through; the
                                // other two open on their own.
                                .default_open(kind != Branch::System)
                                .show(ui, |ui| {
                                    for index in members {
                                        let group = &groups[index];
                                        let selected = *selected_group == Some(index);
                                        let gone = kind == Branch::Gone;

                                        let response = ui
                                            .horizontal(|ui| {
                                                // The program's own icon, the
                                                // same picture the table shows
                                                // for its entries. Falls back
                                                // to the executable itself,
                                                // which is what Windows would
                                                // draw anyway.
                                                let reference = group
                                                    .icon_ref
                                                    .clone()
                                                    .unwrap_or_else(|| format!("{},0", group.key));
                                                let texture = icons.get(&reference).clone();
                                                ui.add(egui::Image::new(
                                                    egui::load::SizedTexture::new(
                                                        texture.id(),
                                                        egui::vec2(16.0, 16.0),
                                                    ),
                                                ));

                                                let label = format!(
                                                    "{:>3}×  {}",
                                                    group.entry_count(),
                                                    group.display_name
                                                );
                                                // Red says "this runs
                                                // something that is no longer
                                                // here" — a menu item that
                                                // fails only when clicked.
                                                let text = if gone {
                                                    egui::RichText::new(label)
                                                        .color(ui.visuals().error_fg_color)
                                                } else {
                                                    egui::RichText::new(label)
                                                };
                                                ui.selectable_label(selected, text)
                                            })
                                            .inner;

                                        if response.clicked() {
                                            clicked = Some(index);
                                        }
                                        // The full path is long and only
                                        // occasionally wanted, so it lives in
                                        // the tooltip — and so does the reason
                                        // for the red, which in the list is
                                        // said by the colour alone.
                                        if gone {
                                            response.on_hover_text(format!(
                                                "{}\n{}",
                                                group.key, tr.badge_uninstalled
                                            ));
                                        } else {
                                            response.on_hover_text(&group.key);
                                        }
                                    }
                                });
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
            anchor,
            glyphs,
            tr,
            bench,
            sort,
            ..
        } = self;
        let glyphs = &*glyphs;
        let mut new_sort: Option<SortBy> = None;
        // Filled by a double click or the context menu, acted on after the
        // table is done: `self` is borrowed field by field in here.
        let mut open: Option<ContextEntry> = None;
        let mut menu_action: Option<Action> = None;

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
                    let reference = visible_rows[row.index()].clone();
                    let Some(entry) = resolve(scan, &reference) else {
                        return;
                    };
                    let depth = reference.path.len();

                    // Must precede the first cell; it only affects cells added
                    // after the call.
                    row.set_selected(selected.contains(&reference));

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
                    // A click selects the row that was clicked — a submenu
                    // child included. It used to select the child's parent,
                    // because the selection could only hold whole entries.
                    if response.clicked() {
                        let keys = response.ctx.input(|i| i.modifiers);
                        apply_click(
                            selected,
                            anchor,
                            visible_rows,
                            &reference,
                            keys.ctrl,
                            keys.shift,
                        );
                        // The focus follows the click even when the anchor
                        // stays put, so the detail pane shows the row under the
                        // pointer rather than the one a range started at.
                        *focused = Some(reference.clone());
                    }

                    // Double click opens the entry — the way everybody tries
                    // first, and for a while the only one that did anything.
                    if response.double_clicked() {
                        open = Some(entry.clone());
                    }

                    // Right-clicking a row that is not part of the selection
                    // makes it the selection first. Otherwise the menu would
                    // list what this row can do and then do it to twenty other
                    // rows, which is how Explorer would never behave.
                    if response.secondary_clicked() && !selected.contains(&reference) {
                        selected.clear();
                        selected.insert(reference.clone());
                        *anchor = Some(reference.clone());
                        *focused = Some(reference.clone());
                    }

                    response.context_menu(|ui| {
                        let alone = selected.len() <= 1;
                        // Said out loud, because the menu is opened on one row
                        // and acts on all of them.
                        if !alone {
                            ui.label(
                                egui::RichText::new(
                                    tr.fmt_selected_count
                                        .replace("{}", &selected.len().to_string()),
                                )
                                .weak()
                                .small(),
                            );
                            ui.separator();
                        }

                        // Looking at an entry is a single-entry affair: the
                        // editor shows one form, not twenty.
                        if alone {
                            if ui
                                .button(labelled(glyphs.inspect, tr.ctx_open_entry))
                                .on_hover_text(tr.tip_ctx_open_entry)
                                .clicked()
                            {
                                open = Some(entry.clone());
                                ui.close();
                            }
                            ui.separator();
                        }

                        for action in actions_for(entry, alone) {
                            let label =
                                labelled(menu_glyph(&action, glyphs), &menu_label(&action, tr));
                            let delete = matches!(action, Action::Delete);
                            if delete {
                                ui.separator();
                            }
                            let text = match delete {
                                true => {
                                    egui::RichText::new(label).color(ui.visuals().error_fg_color)
                                }
                                false => egui::RichText::new(label),
                            };
                            // The same sentence the switch in the bar carries,
                            // so the menu is another way in and not a second
                            // vocabulary.
                            if ui
                                .button(text)
                                .on_hover_text(action_tip(&action, tr))
                                .clicked()
                            {
                                menu_action = Some(action);
                                ui.close();
                            }
                        }
                    });
                });
            });

        if let Some(entry) = open {
            // The form filled in from what is really in the registry, and
            // locked: this shows an entry, it does not change one.
            self.dialog = Some(Dialog::Editor {
                entry: Box::new(NewEntry::from_scanned(&entry)),
                recorded: Vec::new(),
                existing: Some(entry.registry_path.clone()),
            });
        }

        // Through the same `propose` the bar uses, so a menu line and a switch
        // reach the same confirmation, the same backup and the same elevation
        // path. The menu is another way in, not a second implementation.
        if let Some(action) = menu_action {
            self.propose(action);
        }

        if let Some(column) = new_sort {
            // Clicking the column that is already active turns the order
            // around, which is what every file list does.
            // Ascending, descending, and back to the order the rows were
            // collected in. Without the third step the natural order — which
            // carries meaning in the file type tab — would be gone for the
            // rest of the session after one curious click.
            self.sort = match self.sort {
                (active, true) if active == column => (column, false),
                (active, false) if active == column => (SortBy::Natural, true),
                _ => (column, true),
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

                // Nothing selected means nobody has worked with this window
                // yet — `rebuild_visible` focuses the first row by itself, so
                // an empty detail pane is not the signal it looks like. The
                // three steps go away at the first click or arrow key, because
                // both of those select, and that is exactly how long they are
                // worth reading.
                if self.selected.is_empty() {
                    intro(ui, self.tr);
                }

                // Through `resolve`, so the detail pane shows the submenu
                // child that was clicked rather than its parent.
                let Some(entry) = self
                    .focused
                    .as_ref()
                    .and_then(|row| self.scan.as_ref().and_then(|s| resolve(s, row)))
                else {
                    ui.label(self.tr.detail_nothing_selected);
                    return;
                };

                // Looked up in the groups rather than on disk: the file system
                // has no business in a frame path, and the answer was worked
                // out once when the view was built.
                let gone = entry.program_key.as_deref().is_some_and(|key| {
                    let resolved = crate::program::identity::absolute_path(key).to_lowercase();
                    self.groups
                        .iter()
                        .any(|g| g.key == resolved && g.presence == Presence::Missing)
                });

                // The file this entry runs, if it can be named: what the button
                // below opens, and the same resolution the program grouping
                // uses, so both mean the same file.
                let target: Option<std::path::PathBuf> = entry
                    .program_key
                    .as_deref()
                    .map(crate::program::identity::absolute_path)
                    .map(|key| crate::registry::mui::expand_env(&key))
                    .map(std::path::PathBuf::from)
                    .filter(|path| path.is_absolute());

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(self.tr.detail_display_name)
                                    .weak()
                                    .small(),
                            );
                            // Straight to the file in Explorer. The path is in
                            // the pane already, but reading it and finding it
                            // are two different jobs — and this is the shortest
                            // way to "what actually is that program".
                            if let Some(path) = &target
                                && folder_button(
                                    ui,
                                    &mut self.icons,
                                    &self
                                        .tr
                                        .fmt_tip_show_in_explorer
                                        .replace("{}", &path.display().to_string()),
                                )
                                && let Err(error) = elevation::show_in_explorer(path)
                            {
                                self.dialog = Some(Dialog::Error(format!("{error:#}")));
                            }
                        });
                        ui.add(
                            egui::Label::new(&entry.display_name)
                                .selectable(true)
                                .wrap(),
                        );
                        // Here the sentence, in the list only the colour: this
                        // is where there is room to say what a red row means.
                        if gone {
                            ui.colored_label(
                                ui.visuals().error_fg_color,
                                self.tr.detail_program_gone,
                            );
                        }
                        field(ui, self.tr.detail_registry_path, &entry.registry_path);
                        if let Some(raw) = &entry.raw_display {
                            field(ui, self.tr.detail_raw_value, raw);
                        }
                        if let Some(icon) = &entry.icon_ref {
                            // The picture beside the reference that names it.
                            // The text alone — `shell32.dll,-244` — says which
                            // file and which index, and nothing at all about
                            // what the menu will show.
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(self.tr.detail_icon).weak().small());
                                let texture = self.icons.get(icon).clone();
                                ui.add(egui::Image::new(egui::load::SizedTexture::new(
                                    texture.id(),
                                    egui::vec2(16.0, 16.0),
                                )));
                            });
                            ui.add(egui::Label::new(icon).selectable(true).wrap());
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

                        // What the symbols in the table's flag column mean, for
                        // this entry and no other. A lock and an arrow say that
                        // something is special about a row; only here is there
                        // room to say what.
                        let flags = explained_flags(entry, self.tr);
                        if !flags.is_empty() {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.label(egui::RichText::new(self.tr.detail_flags).weak().small());
                            for (name, meaning) in flags {
                                ui.add_space(2.0);
                                ui.label(egui::RichText::new(name).strong());
                                ui.add(egui::Label::new(meaning).wrap());
                            }
                        }
                    });
            });
    }

    fn backup_list(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        let mut start_full = false;
        ui.horizontal(|ui| {
            ui.heading(self.tr.tab_backups);
            if ui.button(self.tr.btn_rescan).clicked() {
                self.reload_backups();
            }
            let running = self.full_backup_rx.is_some();
            if ui
                .add_enabled(!running, egui::Button::new(self.tr.btn_backup_all))
                .on_hover_text(self.tr.tip_backup_all)
                .on_disabled_hover_text(self.tr.tip_backup_all)
                .clicked()
            {
                start_full = true;
            }
            if running {
                ui.spinner();
            }
        });
        if start_full {
            self.full_backup(ui.ctx());
        }
        ui.separator();

        if let Some(error) = &self.backup_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }

        if self.backups.is_empty() {
            ui.label(self.tr.msg_backup_first);
            return;
        }

        let mut restore: Option<std::path::PathBuf> = None;

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
                        ui.horizontal(|ui| {
                            ui.label(path.display().to_string());
                            // The delete tooltip has been promising this
                            // button since it was written; until now the only
                            // way back was the result dialog of the very
                            // action one wanted to undo, or the command line.
                            if ui
                                .button(self.tr.btn_restore)
                                .on_hover_text(self.tr.tip_restore)
                                .clicked()
                            {
                                restore = Some(path.clone());
                            }
                        });
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

        // Applied after the loop, which is walking the list this replaces.
        if let Some(path) = restore {
            match backup::restore(&path) {
                Ok(count) => {
                    elevation::notify_shell();
                    self.dialog = Some(Dialog::Note(
                        self.tr.fmt_restored.replace("{}", &count.to_string()),
                    ));
                    self.filter_dirty = true;
                }
                Err(error) => self.dialog = Some(Dialog::Error(format!("{error:#}"))),
            }
        }
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
    field_width: f32,
    icons: &mut IconCache,
) {
    egui::Grid::new("fav-program")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr.fav_path);
            ui.horizontal(|ui| {
                let mut text = path.to_string_lossy().to_string();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(field_width - 26.0)
                            .hint_text(HINT_PROGRAM),
                    )
                    .changed()
                {
                    // Pasted paths often arrive wrapped in quotes, and a quoted
                    // path resolves to nothing.
                    *path = std::path::PathBuf::from(text.trim().trim_matches('"'));
                }
                // The same picker as in the entry editor. Here the bare path
                // is what the field holds — the arguments have their own line
                // below — so nothing is appended to it.
                if folder_button(ui, icons, tr.tip_pick_program)
                    && let Some(picked) =
                        crate::filedialog::pick_file(None, &crate::filedialog::PROGRAMS, &text)
                {
                    *path = picked;
                }
            });
            ui.end_row();

            ui.label(tr.fav_args);
            ui.add(
                egui::TextEdit::singleline(args)
                    .desired_width(field_width)
                    .hint_text(HINT_ARGS),
            );
            ui.end_row();
        });
    ui.small(tr.fav_args_hint);
}

fn web_form(ui: &mut Ui, tr: &'static Strings, web: &mut WebTool, field_width: f32) {
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
                        .desired_width(field_width)
                        .hint_text(HINT_URL),
                );
            });
        }
        WebMode::Upload(upload) => upload_form(ui, tr, upload, field_width),
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

fn upload_form(ui: &mut Ui, tr: &'static Strings, upload: &mut Upload, field_width: f32) {
    egui::Grid::new("fav-upload")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr.fav_endpoint);
            ui.add(
                egui::TextEdit::singleline(&mut upload.endpoint)
                    .desired_width(field_width)
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
            if ui.small_button("\u{00d7}").clicked() {
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

/// The category a new entry should start in.
///
/// In order of how specific the signal is: the row last clicked, then the
/// selection in the tree of the tab being looked at, then the last category
/// chosen at all. Somebody who has just picked `.png` and presses "New" means
/// `.png`, and having to say so again in a combo box is the kind of small
/// insult that makes a form feel stupid.
///
/// File type categories that cannot be written to are mapped to the one that
/// can: a ProgID is shared by several extensions, so an entry meant "for this
/// kind of file" belongs under the extension it was reached from.
fn category_for_new_entry(
    focused: Option<&ContextEntry>,
    tab: Tab,
    selected_ext: Option<&str>,
    selected_category: Option<&Category>,
) -> Category {
    if let Some(entry) = focused
        && let Some(category) = creatable_category(&entry.category)
    {
        return category;
    }

    if tab == Tab::FileTypes
        && let Some(ext) = selected_ext
    {
        return Category::ExtAssoc(ext.to_string());
    }

    selected_category
        .cloned()
        // The most common thing anybody adds an entry for, and the one place
        // where a guess is harmless: it is visible in the form and one click
        // to change.
        .unwrap_or(Category::Directory)
}

/// The nearest category an entry can actually be created in.
fn creatable_category(category: &Category) -> Option<Category> {
    match category {
        Category::PerceivedType(_) | Category::ExtAssoc(_) => Some(category.clone()),
        // Both of these name one extension and cannot be written to directly,
        // but `SystemFileAssociations\<ext>` means the same thing to the user
        // and is the place this program creates entries.
        Category::ProgId { from_ext, .. } => Some(Category::ExtAssoc(from_ext.clone())),
        Category::ExtDirect(ext) => Some(Category::ExtAssoc(ext.clone())),
        base if Category::BASE.contains(base) => Some(base.clone()),
        _ => None,
    }
}

/// A category as it should read in a picker.
///
/// The seven base categories have names; a file type says itself.
fn category_choice_label(category: &Category, tr: &'static Strings) -> String {
    match category.applies_to_label() {
        label if label.is_empty() => category_label(category, tr).to_string(),
        label => label,
    }
}

/// Where an entry turns up, in words rather than registry shorthand.
///
/// `*` means "on every file" and `Folder` means "on folders"; neither says so
/// to anyone who has not read the documentation. For a file type entry the
/// answer is the type itself, which is the difference that matters when one
/// program registers itself twenty times.
fn appears_on(entry: &ContextEntry, tr: &'static Strings) -> String {
    category_choice_label(&entry.category, tr)
}

/// A check result, in the language on screen.
///
/// The checking modules report a cause rather than a sentence — they also feed
/// the console, which has no language setting — so the sentence is built here.
/// Every glyph the interface draws that is not part of a translated string.
///
/// A list rather than a comment, because `every_glyph_the_window_draws_is_in
/// _a_font_it_loads` checks it: a glyph no loaded font carries comes out as an
/// empty box, and eight of those were on screen before two of them were
/// noticed. Anyone reaching for a new symbol adds it here.
///
/// Test-only because the buttons carry their own literals — a list the code
/// indexed into would be less readable at every call site than the character
/// itself, and it is the *checking* that has to be complete, not the plumbing.
///
/// The padlock is in here late: `badges` had been drawing it since the table
/// existed without it ever being listed, so the one glyph on every read-only
/// row was the one glyph nothing checked.
#[cfg(test)]
const UI_GLYPHS: &str = "\u{2192}\u{2191}\u{2193}\u{00d7}\u{21b3}\u{25b4}\u{25b8}\u{25be}\u{00b7}\u{2026}\u{21e7}\u{2713}\u{2717}\u{2195}\u{1f441}\u{1f6ab}\u{1f512}";

/// What Windows substitutes in a command line, and what each one means.
///
/// The list is short on purpose: these five are what turns up in the 3118 real
/// command lines this machine carries. `%*`, `%2`… exist as well and are for
/// verbs invoked with several arguments, which a context menu never does.
type Placeholder = (&'static str, fn(&'static Strings) -> &'static str);
const PLACEHOLDERS: &[Placeholder] = &[
    ("%1", |tr| tr.ph_one),
    ("%L", |tr| tr.ph_long),
    ("%V", |tr| tr.ph_verb),
    ("%W", |tr| tr.ph_working),
    ("%D", |tr| tr.ph_desktop),
];

/// One working command line per shape of category, and what it is for.
///
/// Not translated: a command line is a command line. What differs between the
/// categories is which placeholder carries the path, and that difference is
/// exactly what the entries below spell out — `%1` is empty on a background
/// click, which is the one mistake `create::check` warns about by name.
const EXAMPLES: &[Placeholder] = &[
    (r#""C:\Windows\System32\notepad.exe" "%1""#, |tr| {
        tr.help_example_file
    }),
    (r#""C:\Windows\System32\cmd.exe" /k cd /d "%V""#, |tr| {
        tr.help_example_background
    }),
    (
        r#"explorer "https://github.com/corgan2222/context-manager""#,
        |tr| tr.help_example_url,
    ),
];

/// Windows' own folder icon, for the buttons that open one.
///
/// The emoji `📁` was there first and came out as a pale outline from the
/// fallback font — beside real, coloured shell icons in the same list it
/// looked like a rendering fault. This is the same picture Explorer uses,
/// pulled through the same extractor as every other icon here: resource 4 of
/// `shell32.dll` is the plain closed folder.
const FOLDER_ICON: &str = r"%SystemRoot%\system32\shell32.dll,-4";

/// The three branches of the program tree.
///
/// An order rather than a filter: a program whose file is gone is a finding
/// and belongs on top, Windows' own components belong at the bottom because
/// they are the ones nobody came looking for, and everything else is the
/// middle — which is where the list is actually read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Branch {
    Gone,
    Own,
    System,
}

impl Branch {
    const ALL: [Branch; 3] = [Branch::Gone, Branch::Own, Branch::System];

    fn of(group: &ProgramGroup) -> Branch {
        // "Gone" wins over "system": a Windows component that is not on disk
        // is the more surprising of the two facts.
        if group.presence == Presence::Missing {
            Branch::Gone
        } else if group.is_system {
            Branch::System
        } else {
            Branch::Own
        }
    }

    fn label(self, tr: &'static Strings) -> &'static str {
        match self {
            Branch::Gone => tr.grp_gone_programs,
            Branch::Own => tr.grp_own_programs,
            Branch::System => tr.grp_system_programs,
        }
    }

    /// Stable id for the collapsing header, so opening one does not reshuffle
    /// the state of the others after a rescan.
    fn salt(self) -> &'static str {
        match self {
            Branch::Gone => "programs-gone",
            Branch::Own => "programs-own",
            Branch::System => "programs-system",
        }
    }
}

/// What stands in the title bar: the name in the chosen language, then the
/// version.
///
/// The version belongs where the window can be identified without opening
/// anything — a screenshot in a report says which build it came from. Not
/// translated and not prefixed with a `v`: the number is the same in every
/// language, and `1.0.0` reads as a version without help.
fn window_title(tr: &'static Strings) -> String {
    format!("{} {}", tr.app_title, crate::VERSION)
}

/// A small button carrying Windows' folder icon. Returns whether it was hit.
///
/// One function for all of them, because there are four by now — the detail
/// pane, the command field, the icon field — and a 16-pixel button assembled
/// three times would drift apart in three directions.
fn folder_button(ui: &mut Ui, icons: &mut IconCache, tooltip: &str) -> bool {
    let texture = icons.get(FOLDER_ICON).clone();
    ui.add(egui::Button::image(egui::load::SizedTexture::new(
        texture.id(),
        egui::vec2(16.0, 16.0),
    )))
    .on_hover_text(tooltip)
    .clicked()
}

/// A fresh, empty row of the submenu list.
fn empty_child() -> NewChild {
    NewChild {
        // Derived from the position in the list every frame, so there is
        // nothing to fill in here (see `children_editor`).
        key_name: String::new(),
        display_name: String::new(),
        command: String::new(),
        icon: None,
    }
}

/// The rows of a submenu: name, command, icon, and the order.
///
/// Its own function because the editor dialog is long enough already, and
/// because the order needs three deferred actions — a list cannot be
/// rearranged while it is being iterated over.
fn children_editor(
    ui: &mut Ui,
    children: &mut Vec<NewChild>,
    icons: &mut IconCache,
    tr: &'static Strings,
    field_width: f32,
) {
    let mut move_up = None;
    let mut move_down = None;
    let mut remove = None;
    let count = children.len();

    // Three fields on one line, and they have to share what the window gives
    // them: fixed widths left half the dialog empty once it was dragged wide,
    // while `c:\windows\system…` still did not fit. What is subtracted is the
    // fixed furniture — two pickers, the preview icon, three buttons, gaps.
    let usable = (field_width - 210.0).max(300.0);
    let name_width = usable * 0.28;
    let command_width = usable * 0.45;
    let icon_width = usable * 0.27;

    for (index, child) in children.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut child.display_name)
                    .desired_width(name_width)
                    .hint_text(tr.editor_child_name_hint),
            );
            ui.add(
                egui::TextEdit::singleline(&mut child.command)
                    .desired_width(command_width)
                    .hint_text(HINT_COMMAND),
            );
            // The same two pickers the single-entry form has. A submenu is
            // where paths are typed most often — one per child — so leaving
            // them out here was leaving them out where they were needed most.
            if folder_button(ui, icons, tr.tip_pick_program)
                && let Some(path) =
                    crate::filedialog::pick_file(None, &crate::filedialog::PROGRAMS, &child.command)
            {
                child.command = format!("\"{}\" \"%1\"", path.display());
            }

            let mut icon = child.icon.clone().unwrap_or_default();
            ui.add(
                egui::TextEdit::singleline(&mut icon)
                    .desired_width(icon_width)
                    .hint_text(HINT_ICON),
            );
            let mut icon = icon.trim().to_string();
            if folder_button(ui, icons, tr.tip_pick_icon)
                && let Some(path) =
                    crate::filedialog::pick_file(None, &crate::filedialog::ICONS, &icon)
            {
                // `,0` for the same reason as everywhere else: a reference is
                // split at its last comma, so a path containing one would lose
                // everything behind it.
                icon = format!("{},0", path.display());
            }
            // Same preview as the entry's own icon field, for the same
            // reason: a typo in `shell32.dll,-244` is invisible otherwise.
            if !icon.is_empty() {
                let texture = icons.get(&icon).clone();
                ui.add(egui::Image::new(egui::load::SizedTexture::new(
                    texture.id(),
                    egui::vec2(16.0, 16.0),
                )));
            }
            child.icon = (!icon.is_empty()).then_some(icon);

            if ui
                .add_enabled(index > 0, egui::Button::new("\u{2191}"))
                .on_hover_text(tr.tip_child_up)
                .clicked()
            {
                move_up = Some(index);
            }
            if ui
                .add_enabled(index + 1 < count, egui::Button::new("\u{2193}"))
                .on_hover_text(tr.tip_child_down)
                .clicked()
            {
                move_down = Some(index);
            }
            // The last one stays: an empty submenu is not a state this form
            // can write, and "single entry" above is the way back.
            if ui
                .add_enabled(count > 1, egui::Button::new("\u{00d7}"))
                .on_hover_text(tr.tip_child_remove)
                .clicked()
            {
                remove = Some(index);
            }
        });
    }

    if let Some(index) = move_up {
        children.swap(index, index - 1);
    }
    if let Some(index) = move_down {
        children.swap(index, index + 1);
    }
    if let Some(index) = remove {
        children.remove(index);
    }

    if ui.button(tr.editor_child_add).clicked() {
        children.push(empty_child());
    }

    // The key name is the order — the registry hands subkeys back in
    // alphabetical order whatever order they were written in — so it is
    // derived here rather than typed, and re-derived after every move.
    for (index, child) in children.iter_mut().enumerate() {
        child.key_name = create::suggest_child_key_name(index, &child.display_name);
    }
}

fn fault_text(fault: &Fault, tr: &'static Strings) -> String {
    match fault {
        Fault::MissingKeyName => tr.fault_key_name.to_string(),
        Fault::BackslashInKeyName => tr.fault_backslash.to_string(),
        Fault::MissingDisplayName => tr.fault_display_name.to_string(),
        Fault::MissingCommand => tr.fault_command.to_string(),
        Fault::PercentOneInBackground => tr.fault_percent_one.to_string(),
        Fault::AmpersandInDisplayName => tr.fault_ampersand.to_string(),
        Fault::UnusualPosition(value) => tr.fmt_fault_position.replace("{}", value),
        Fault::CommandBesideSubmenu => tr.fault_command_in_submenu.to_string(),
        Fault::ChildMissingDisplayName(n) => tr.fmt_fault_child_name.replace("{}", &n.to_string()),
        Fault::ChildMissingCommand(n) => tr.fmt_fault_child_command.replace("{}", &n.to_string()),
        Fault::DuplicateChildKeyName(name) => tr.fmt_fault_child_duplicate.replace("{}", name),
        Fault::CategoryNotCreatable => tr.fault_category.to_string(),
        Fault::UnusableKeyName => tr.fault_key_name_refused.to_string(),
    }
}

/// What is wrong with a favourite, in the language on screen.
///
/// The counterpart to [`fault_text`]. Until 2026-08-15 this dialog showed the
/// bilingual string straight from `favourites`, so every reader read half a
/// message they did not need — the same fault that was fixed for the entry
/// editor and the same cure.
fn fav_fault_text(fault: &favourites::Fault, tr: &'static Strings) -> String {
    use favourites::Fault;
    match fault {
        Fault::MissingName => tr.fault_fav_name.to_string(),
        Fault::MissingPath => tr.fault_fav_path.to_string(),
        Fault::FileNotFound(path) => tr.fmt_fault_fav_missing_file.replace("{}", path),
        Fault::MissingAddress => tr.fault_fav_address.to_string(),
        Fault::InsecureAddress => tr.fault_fav_insecure.to_string(),
        Fault::NotHttps => tr.fault_fav_not_https.to_string(),
        Fault::NoPlaceholder => tr.fault_fav_placeholder.to_string(),
    }
}

/// The name of an action in the language on screen.
///
/// `Action::label` stays German on purpose — it names the backup directory,
/// and a directory whose name changed with the interface language would be a
/// small archaeology problem later.
fn action_label(action: &Action, tr: &'static Strings) -> &'static str {
    match action {
        Action::Hide => tr.act_hide,
        Action::Show => tr.act_show,
        Action::ShiftOnly => tr.act_shift_only,
        Action::AlwaysShow => tr.act_always_show,
        Action::SetPosition(_) => tr.act_position,
        Action::Block => tr.act_block,
        Action::Unblock => tr.act_unblock,
        Action::Delete => tr.act_delete,
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
        Category::CommandStore => tr.cat_command_store,
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
fn sort_key(entry: &ContextEntry, column: SortBy, tr: &'static Strings) -> String {
    match column {
        // Never asked for: `rebuild_visible` skips sorting entirely for it.
        SortBy::Natural => String::new(),
        SortBy::Name => entry.display_name.to_lowercase(),
        SortBy::Kind => entry.kind.type_label().to_string(),
        SortBy::Scope => entry.scope.label().to_string(),
        // By what the column actually shows. Sorting by a hidden value is
        // the kind of thing that makes a list look broken: the reader sees
        // "Alle Dateien" and an order that has nothing to do with it.
        SortBy::AppliesTo => appears_on(entry, tr).to_lowercase(),
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

/// What this program is, in three steps and one promise.
///
/// It sits in the detail pane rather than in a window of its own: that pane is
/// where the eye goes before anything has been clicked, it is already
/// scrollable, and a dialog on first start would have to be dismissed by
/// people who never needed it.
fn intro(ui: &mut Ui, tr: &'static Strings) {
    ui.label(egui::RichText::new(tr.intro_title).strong());
    ui.add_space(6.0);
    for step in [tr.intro_step_one, tr.intro_step_two, tr.intro_step_three] {
        ui.add(egui::Label::new(step).wrap());
        ui.add_space(4.0);
    }
    ui.add_space(4.0);
    ui.add(egui::Label::new(egui::RichText::new(tr.intro_safety).weak().small()).wrap());
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(4.0);
}

/// The line above the switches: what is selected, or what to do about that.
///
/// It replaces a bare "0 ausgewählt". The count was true and useless — it sat
/// at the far left of a bar full of grey buttons and never connected the two,
/// so the reason everything was disabled went unread.
fn selection_summary(tr: &'static Strings, state: &SelectionState) -> String {
    if state.count == 0 {
        return tr.hint_no_selection.to_string();
    }
    if state.read_only > 0 {
        // Said here rather than discovered in the report afterwards: these
        // rows stay in the plan on purpose, and knowing beforehand that some
        // of them need administrator rights is the difference between a
        // surprise and a decision.
        return tr
            .fmt_selection_readonly
            .replacen("{}", &state.count.to_string(), 1)
            .replacen("{}", &state.read_only.to_string(), 1);
    }
    tr.fmt_selected_count
        .replace("{}", &state.count.to_string())
}

/// The four state switches, and the action a click on one of them asks for.
///
/// Each group is one axis with the selection sitting somewhere on it. Where
/// there used to be a button per direction — `Hide` here, `Show` a row below in
/// a smaller size — there is now one group per axis that also answers the
/// question the buttons never did: which end are these entries at right now?
fn switch_groups(
    ui: &mut Ui,
    tr: &'static Strings,
    glyphs: &Glyphs,
    state: &SelectionState,
) -> Option<Action> {
    let mut wanted = None;

    // With nothing selected every group is off for the same reason, so they
    // all give the same answer when hovered.
    let needs_rows = (state.count == 0).then_some(tr.tip_needs_selection);

    if let Some(index) = switch_group(
        ui,
        tr.group_visibility,
        mixed_marker(tr, state.hidden == Agreement::Mixed),
        &group_tip(tr.group_visibility, tr.tip_group_visibility),
        needs_rows,
        &[
            (
                labelled(glyphs.visible, tr.seg_visible),
                state.hidden == Agreement::Same(false),
            ),
            (
                labelled(glyphs.hidden, tr.seg_hidden),
                state.hidden == Agreement::Same(true),
            ),
        ],
    ) {
        wanted = Some(match index {
            0 => Action::Show,
            _ => Action::Hide,
        });
    }

    ui.separator();

    if let Some(index) = switch_group(
        ui,
        tr.group_shift,
        mixed_marker(tr, state.extended == Agreement::Mixed),
        &group_tip(tr.group_shift, tr.tip_group_shift),
        needs_rows,
        &[
            (
                labelled(glyphs.always, tr.seg_always),
                state.extended == Agreement::Same(false),
            ),
            (
                labelled(glyphs.shift_only, tr.seg_shift_only),
                state.extended == Agreement::Same(true),
            ),
        ],
    ) {
        wanted = Some(match index {
            0 => Action::AlwaysShow,
            _ => Action::ShiftOnly,
        });
    }

    ui.separator();

    // Blocking is a COM-handler mechanism, so a selection of static verbs
    // cannot use it. The old bar offered the button anyway and answered with
    // an error window after the click; the reason is known before it.
    let blocking = needs_rows.or((state.blockable == 0).then_some(tr.tip_needs_clsid));
    if let Some(index) = switch_group(
        ui,
        tr.group_systemwide,
        mixed_marker(tr, state.blocked == Agreement::Mixed),
        &group_tip(tr.group_systemwide, tr.tip_group_systemwide),
        blocking,
        &[
            (
                labelled(glyphs.free, tr.seg_free),
                state.blocked == Agreement::Same(false),
            ),
            (
                labelled(glyphs.blocked, tr.seg_blocked),
                state.blocked == Agreement::Same(true),
            ),
        ],
    ) {
        wanted = Some(match index {
            0 => Action::Unblock,
            _ => Action::Block,
        });
    }

    ui.separator();

    // Both values verified on Windows 10 by writing probe verbs and
    // photographing a real right-click: an entry with Top rises above
    // alphabetically earlier siblings, one with Bottom sinks below everything.
    // Three coarse blocks are all Windows actually gives.
    let at = |value: Option<&str>| state.position == Agreement::Same(value.map(str::to_string));
    if let Some(index) = switch_group(
        ui,
        tr.group_position,
        mixed_marker(tr, state.position == Agreement::Mixed),
        &group_tip(tr.group_position, tr.tip_position),
        needs_rows,
        &[
            (labelled(glyphs.no_position, tr.pos_default), at(None)),
            (labelled(glyphs.top, tr.pos_top), at(Some("Top"))),
            (labelled(glyphs.bottom, tr.pos_bottom), at(Some("Bottom"))),
        ],
    ) {
        wanted = Some(Action::SetPosition(match index {
            1 => Some("Top".to_string()),
            2 => Some("Bottom".to_string()),
            _ => None,
        }));
    }

    wanted
}

/// One group of segments, allocated as a unit so it cannot break in half.
///
/// No frame around it: the segments carry an icon each, and a box drawn around
/// every pair turned the bar into four boxes competing with the buttons inside
/// them. What holds a group together now is that it is allocated in one piece,
/// with a plain separator between groups.
///
/// A lit segment is where the selection already is, so clicking it would ask
/// for a change that has already happened. It stays clickable rather than
/// being disabled — with a mixed selection the lit segment is the one worth
/// clicking, to bring the rest into line.
fn switch_group(
    ui: &mut Ui,
    title: &str,
    mixed: Option<&str>,
    tip: &str,
    reason: Option<&str>,
    segments: &[(String, bool)],
) -> Option<usize> {
    // A wrapping row breaks between widgets whose width it is told in advance,
    // and a group of loose buttons never tells it one — each button decides for
    // itself and the group can come apart across two lines. Measured at 1267
    // points before this: the last two groups hung over the right edge instead
    // of moving down. Reserving the measured size first gives the row something
    // to decide with, and the height has to be part of it — with zero the
    // groups came out as a staircase, each starting lower than the last.
    let needed = group_width(ui, title, mixed, segments);
    let height = group_height(ui);
    let mut clicked = None;
    ui.allocate_ui_with_layout(
        egui::vec2(needed, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            // What this group is for, in front of it. Two icons and two words
            // say what the choice is between; they do not say what is being
            // chosen, and "Immer / Nur mit Umschalt" without "Umschalttaste:"
            // in front is a question rather than a control.
            ui.label(egui::RichText::new(format!("{title}:")).weak());
            // Only when the rows disagree: without the word, a mixed selection —
            // where no segment lights up — looks exactly like an empty one.
            if let Some(mixed) = mixed {
                ui.label(egui::RichText::new(mixed).weak().small());
            }
            for (index, (label, current)) in segments.iter().enumerate() {
                // `Button::selectable`, not `SelectableLabel`: egui 0.36 folded
                // the second into the first, and the tab row above takes the
                // same shape through `ui.selectable_label`.
                let response = ui
                    .add_enabled(
                        reason.is_none(),
                        egui::Button::selectable(*current, label.as_str()),
                    )
                    .on_hover_text(tip)
                    // A greyed-out control that explains nothing is the thing
                    // this whole bar was rebuilt to get rid of.
                    .on_disabled_hover_text(reason.unwrap_or(tip));
                if response.clicked() {
                    clicked = Some(index);
                }
            }
        },
    );
    clicked
}

/// How much room a switch group is about to need.
///
/// Measured against the fonts actually loaded rather than guessed at with a
/// constant: the German labels are longer than the English ones, the user can
/// switch language at runtime, and a wrong guess here shows up as a group
/// hanging over the right edge.
fn group_width(ui: &Ui, title: &str, mixed: Option<&str>, segments: &[(String, bool)]) -> f32 {
    let body = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::TextStyle::Small.resolve(ui.style());
    // Through the painter rather than `ui.fonts`: laying text out fills a cache
    // and therefore needs `&mut`, which the closure form does not hand out.
    // Nothing is drawn — the galley is measured and dropped.
    let measure = |text: &str, font: &egui::FontId| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
            .size()
            .x
    };

    let spacing = ui.spacing().item_spacing.x;
    let padding = ui.spacing().button_padding.x * 2.0;
    let mut width = measure(&format!("{title}:"), &body)
        + spacing
        + mixed.map_or(0.0, |text| measure(text, &small) + spacing);
    for (label, _) in segments {
        width += spacing + padding + measure(label, &body);
    }
    width + spacing
}

/// How tall a switch group comes out.
fn group_height(ui: &Ui) -> f32 {
    ui.spacing()
        .interact_size
        .y
        .max(ui.text_style_height(&egui::TextStyle::Body))
        + ui.spacing().button_padding.y * 2.0
}

/// The word for a selection that does not agree with itself, or nothing.
fn mixed_marker(tr: &'static Strings, mixed: bool) -> Option<&'static str> {
    mixed.then_some(tr.seg_mixed)
}

/// What a group is about, in front of what it does.
///
/// The name used to stand beside the symbol and now only lives here: with the
/// words gone from the bar, the tooltip is the one place left that says which
/// of the three mechanisms this symbol stands for.
fn group_tip(name: &str, explanation: &str) -> String {
    format!("{name} — {explanation}")
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

/// The flags of an entry, in words, for the detail pane.
///
/// The table has room for a symbol and no more; a lock and an arrow say *that*
/// something is special and not *what*. Only what is actually set is listed —
/// a fixed legend of six lines would be five lines of noise on most entries.
fn explained_flags(
    entry: &ContextEntry,
    tr: &'static Strings,
) -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();

    if entry.read_only {
        out.push((tr.badge_readonly, tr.why_readonly));
    }
    if entry.hidden {
        out.push((tr.badge_hidden, tr.why_hidden));
    }
    if entry.extended {
        out.push((tr.badge_shift, tr.why_extended));
    }
    if let EntryKind::ShellEx { blocked, .. } = &entry.kind {
        if *blocked {
            out.push((tr.badge_blocked, tr.why_blocked));
        }
        // Not a flag but the same question — "why can I not change the text" —
        // and this is where it gets an answer.
        out.push((tr.kind_shellex, tr.why_com_handler));
    }
    if entry.position.is_some() {
        out.push((tr.detail_position, tr.why_position));
    }
    if matches!(
        entry.scope,
        crate::model::Scope::Machine | crate::model::Scope::Machine32
    ) {
        out.push((tr.badge_admin, tr.why_machine));
    }

    out
}

/// Segoe UI for the text, Segoe UI Symbol for everything that is not a letter.
///
/// egui ships its own font, which is immediately recognisable and wrong for a
/// system tool. A failed read leaves the default font rather than panicking
/// (ToDo 9.3).
///
/// The second file is not decoration. Measured on 2026-08-15 against every
/// font this application loads — Segoe UI plus the three egui never removes —
/// **eight of the glyphs the interface draws are in none of them**: `↳ ⇧ ▴ ▸
/// ▾ ✓ ✗ ✕`. Each one was an empty box on screen, and only two of them were
/// ever noticed. `seguisym.ttf` carries all eight, ships with Windows since
/// Vista, and costs 2.4 MB of address space rather than binary size.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut family = Vec::new();

    // Segoe UI Variable only exists from Windows 11 on, so the older file is
    // tried first — it is present everywhere.
    for (name, candidate) in [
        ("segoe", r"C:\Windows\Fonts\segoeui.ttf"),
        ("segoe", r"C:\Windows\Fonts\SegUIVar.ttf"),
        ("segoe-symbol", r"C:\Windows\Fonts\seguisym.ttf"),
    ] {
        // The text face is settled by the first file that reads; the symbol
        // face is a separate name and joins it either way.
        if family.iter().any(|had| had == name) {
            continue;
        }
        let Ok(data) = std::fs::read(candidate) else {
            continue;
        };

        fonts.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(data)),
        );
        family.push(name.to_owned());
    }

    // Feather, through `iconflow`: the pack ships as a TTF whose glyphs sit in
    // the private use area, so it joins the same family as the text faces and a
    // button can carry an icon by putting one character in front of its label.
    //
    // **In front of the system faces, and that is not a detail.** Segoe UI
    // Symbol has glyphs in the private use area too — measured on 2026-08-15:
    // **187 of Feather's 287 codepoints are also in seguisym.ttf**. With the
    // icons behind it, nine of the fifteen buttons in the bar drew a Segoe
    // glyph instead: the trash can came out as a list, the shield as a login
    // arrow. Feather carries nothing outside U+E000..U+E11E, so putting it
    // first costs one failed lookup per ordinary character and no text.
    let mut icon_families = Vec::new();
    for asset in iconflow::fonts() {
        fonts.font_data.insert(
            asset.family.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(asset.bytes)),
        );
        icon_families.push(asset.family.to_owned());
    }
    icon_families.append(&mut family);
    let family = icon_families;

    if family.is_empty() {
        return;
    }

    // In front of egui's own fonts, in this order: a glyph is looked up in
    // Segoe UI first and falls through to the symbol face only when it is not
    // there, so ordinary text keeps the face it had.
    let proportional = fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    for (index, name) in family.into_iter().enumerate() {
        proportional.insert(index, name);
    }

    ctx.set_fonts(fonts);
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
    start_ext: Option<String>,
    theme_probe: bool,
) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // German here and corrected in the first frame, because the
            // settings are not read yet — `sync_title` puts both the language
            // and the version right before anyone can read this one.
            .with_title(window_title(&i18n::DE))
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
                start_ext.clone(),
                theme_probe,
            )))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic;

    fn rows(indices: &[usize]) -> rustc_hash::FxHashSet<Row> {
        indices.iter().map(|index| Row::top(*index)).collect()
    }

    #[test]
    fn the_menu_offers_the_direction_that_would_change_something() {
        let mut scan = synthetic::scan_result(8);

        // A visible verb: hide, shift-only, no blocking, and the two positions
        // it is not already at plus "none" — which it is at, so not that one.
        let verb = &mut scan.entries[0];
        verb.hidden = false;
        verb.extended = false;
        verb.position = None;
        let offered = actions_for(verb, true);
        assert!(offered.contains(&Action::Hide));
        assert!(!offered.contains(&Action::Show));
        assert!(offered.contains(&Action::ShiftOnly));
        assert!(
            !offered.iter().any(|a| matches!(a, Action::Block)),
            "a static verb has no CLSID to block"
        );
        assert!(offered.contains(&Action::SetPosition(Some("Top".into()))));
        assert!(
            !offered.contains(&Action::SetPosition(None)),
            "it is already at no position"
        );
        assert!(offered.contains(&Action::Delete));

        // The same entry hidden and shifted: both directions turn around.
        let hidden = &mut scan.entries[1];
        hidden.hidden = true;
        hidden.extended = true;
        let offered = actions_for(hidden, true);
        assert!(offered.contains(&Action::Show));
        assert!(!offered.contains(&Action::Hide));
        assert!(offered.contains(&Action::AlwaysShow));
        assert!(!offered.contains(&Action::ShiftOnly));

        // Every fourth synthetic entry is a COM handler.
        let handler = &scan.entries[3];
        let offered = actions_for(handler, true);
        assert!(
            offered.contains(&Action::Unblock),
            "entry 3 is blocked, so the way out is offered"
        );
        assert!(!offered.contains(&Action::Block));
        assert!(
            !offered.iter().any(|a| matches!(a, Action::SetPosition(_))),
            "the scanner never reads a position for a shellex key"
        );
    }

    #[test]
    fn a_group_is_not_offered_what_only_makes_sense_for_one() {
        let mut scan = synthetic::scan_result(4);
        // Synthetic entry 0 starts out hidden; this test is about position,
        // so it is put in a known state first.
        scan.entries[0].hidden = false;
        let entry = &scan.entries[0];

        let single = actions_for(entry, true);
        let group = actions_for(entry, false);

        assert!(
            single.iter().any(|a| matches!(a, Action::SetPosition(_))),
            "one entry can be sent to the top"
        );
        assert!(
            !group.iter().any(|a| matches!(a, Action::SetPosition(_))),
            "twenty entries all sent to the top are alphabetical again"
        );
        // What does mean something for a group stays.
        assert!(group.contains(&Action::Hide));
        assert!(group.contains(&Action::Delete));
    }

    #[test]
    fn every_menu_line_carries_the_explanation_its_switch_carries() {
        let scan = synthetic::scan_result(8);
        for entry in &scan.entries {
            for action in actions_for(entry, true) {
                assert!(
                    !action_tip(&action, &i18n::DE).trim().is_empty(),
                    "{action:?} has no tooltip"
                );
            }
        }
    }

    #[test]
    fn clicking_selects_the_way_explorer_does() {
        let visible: Vec<Row> = (0..6).map(Row::top).collect();
        let mut selected = rustc_hash::FxHashSet::default();
        let mut anchor = None;

        // Plain click: this row, and nothing else.
        apply_click(
            &mut selected,
            &mut anchor,
            &visible,
            &visible[1],
            false,
            false,
        );
        assert_eq!(selected, rows(&[1]));
        assert_eq!(anchor, Some(visible[1].clone()));

        // Shift: the span from the anchor, anchor unmoved.
        apply_click(
            &mut selected,
            &mut anchor,
            &visible,
            &visible[4],
            false,
            true,
        );
        assert_eq!(selected, rows(&[1, 2, 3, 4]));
        assert_eq!(
            anchor,
            Some(visible[1].clone()),
            "the anchor stays where the range started"
        );

        // A second Shift-click measures from the same anchor, not from the
        // last row of the previous range — the reason the anchor exists.
        apply_click(
            &mut selected,
            &mut anchor,
            &visible,
            &visible[0],
            false,
            true,
        );
        assert_eq!(selected, rows(&[0, 1]));

        // Ctrl toggles a single row and moves the anchor there.
        apply_click(
            &mut selected,
            &mut anchor,
            &visible,
            &visible[5],
            true,
            false,
        );
        assert_eq!(selected, rows(&[0, 1, 5]));
        assert_eq!(anchor, Some(visible[5].clone()));
        apply_click(
            &mut selected,
            &mut anchor,
            &visible,
            &visible[5],
            true,
            false,
        );
        assert_eq!(selected, rows(&[0, 1]), "clicking it again takes it out");

        // Ctrl and Shift together add a range instead of replacing.
        apply_click(
            &mut selected,
            &mut anchor,
            &visible,
            &visible[3],
            true,
            true,
        );
        assert_eq!(selected, rows(&[0, 1, 3, 4, 5]));
    }

    #[test]
    fn a_shift_click_without_an_anchor_behaves_like_a_plain_one() {
        let visible: Vec<Row> = (0..4).map(Row::top).collect();
        let mut selected = rows(&[0, 1, 2]);
        let mut anchor = None;

        apply_click(
            &mut selected,
            &mut anchor,
            &visible,
            &visible[3],
            false,
            true,
        );

        assert_eq!(selected, rows(&[3]));
        assert_eq!(anchor, Some(visible[3].clone()), "and it sets one");
    }

    #[test]
    fn every_icon_the_bar_asks_for_exists_in_the_pack() {
        // A name the pack does not carry comes back as a space, which on screen
        // is a button whose label starts with a gap — easy to miss and easy to
        // introduce, since the names are strings. The pack is versioned and can
        // rename things between releases, so this is worth a test rather than
        // a careful read.
        let missing: Vec<&str> = Glyphs::NAMES
            .iter()
            .copied()
            .filter(|name| feather(name) == ' ')
            .collect();

        assert!(missing.is_empty(), "not in Feather: {missing:?}");
    }

    #[test]
    fn the_glyph_list_and_the_struct_stay_the_same_length() {
        // `Glyphs::load` walks NAMES in order and fills the fields in the same
        // order. Adding a field without adding its name would silently shift
        // every icon after it by one.
        let glyphs = Glyphs::load();
        let filled = [
            glyphs.select_all,
            glyphs.select_none,
            glyphs.visible,
            glyphs.hidden,
            glyphs.always,
            glyphs.shift_only,
            glyphs.free,
            glyphs.blocked,
            glyphs.top,
            glyphs.bottom,
            glyphs.no_position,
            glyphs.delete,
            glyphs.rescan,
            glyphs.new,
            glyphs.backup,
            glyphs.inspect,
            glyphs.copy,
        ];

        assert_eq!(filled.len(), Glyphs::NAMES.len());
        assert!(
            filled.iter().all(|glyph| *glyph != ' '),
            "every field got a glyph"
        );
        // Feather puts its glyphs in the private use area; anything outside it
        // would mean the lookup returned an ordinary character by accident.
        assert!(
            filled
                .iter()
                .all(|glyph| ('\u{e000}'..='\u{f8ff}').contains(glyph)),
            "all in the private use area"
        );
    }

    #[test]
    fn agreement_needs_something_to_agree_about() {
        assert_eq!(agreement(Vec::<bool>::new()), Agreement::Empty);
        assert_eq!(agreement([true]), Agreement::Same(true));
        assert_eq!(agreement([false, false, false]), Agreement::Same(false));
        assert_eq!(agreement([true, false]), Agreement::Mixed);
    }

    #[test]
    fn an_empty_selection_leaves_every_switch_without_a_current_segment() {
        let scan = synthetic::scan_result(8);
        let state = selection_state(Some(&scan), &rows(&[]));

        assert_eq!(state.count, 0);
        assert_eq!(state.resolved, 0);
        assert_eq!(state.hidden, Agreement::Empty);
        assert_eq!(state.extended, Agreement::Empty);
        assert_eq!(state.blocked, Agreement::Empty);
        assert_eq!(state.position, Agreement::Empty);
    }

    #[test]
    fn a_selection_that_agrees_says_so() {
        let mut scan = synthetic::scan_result(8);
        for entry in &mut scan.entries {
            entry.hidden = true;
            entry.extended = false;
            entry.position = Some("Top".into());
        }

        let state = selection_state(Some(&scan), &rows(&[0, 1, 2]));

        assert_eq!(state.count, 3);
        assert_eq!(state.resolved, 3);
        assert_eq!(state.hidden, Agreement::Same(true));
        assert_eq!(state.extended, Agreement::Same(false));
        assert_eq!(state.position, Agreement::Same(Some("Top".to_string())));
    }

    #[test]
    fn one_dissenter_is_enough_to_light_no_segment() {
        let mut scan = synthetic::scan_result(8);
        for entry in &mut scan.entries {
            entry.hidden = true;
        }
        scan.entries[2].hidden = false;

        let state = selection_state(Some(&scan), &rows(&[0, 1, 2]));

        assert_eq!(state.hidden, Agreement::Mixed);
    }

    #[test]
    fn a_selection_without_a_com_handler_has_no_blocked_state_at_all() {
        let scan = synthetic::scan_result(8);
        // Every fourth synthetic entry is a COM handler; 0..=2 are static
        // verbs, and a static verb has no CLSID to put on the blocked list.
        let verbs = selection_state(Some(&scan), &rows(&[0, 1, 2]));
        assert_eq!(verbs.blockable, 0);
        assert_eq!(
            verbs.blocked,
            Agreement::Empty,
            "no CLSID means no answer, not the answer 'free'"
        );

        // Mixing one in: only that row counts towards the blocking switch.
        let mixed = selection_state(Some(&scan), &rows(&[0, 1, 3]));
        assert_eq!(mixed.blockable, 1);
        assert_eq!(mixed.blocked, Agreement::Same(true), "entry 3 is blocked");
    }

    #[test]
    fn a_row_the_scan_no_longer_knows_is_counted_but_not_read() {
        let scan = synthetic::scan_result(4);
        let state = selection_state(Some(&scan), &rows(&[0, 99]));

        assert_eq!(state.count, 2, "the user did select two rows");
        assert_eq!(state.resolved, 1, "one of them no longer exists");
    }

    #[test]
    fn read_only_rows_are_counted_rather_than_dropped() {
        let mut scan = synthetic::scan_result(4);
        scan.entries[0].read_only = true;
        scan.entries[1].read_only = true;

        let state = selection_state(Some(&scan), &rows(&[0, 1, 2]));

        // Reported, not vetoed: `plan_for_selection` keeps such rows on
        // purpose, and an elevated run can write what this one cannot.
        assert_eq!(state.read_only, 2);
        assert_eq!(state.resolved, 3);
    }

    #[test]
    fn without_a_scan_only_the_count_is_known() {
        let state = selection_state(None, &rows(&[0, 1]));

        assert_eq!(state.count, 2);
        assert_eq!(state.resolved, 0);
        assert_eq!(state.changeable, 0);
        assert_eq!(state.hidden, Agreement::Empty);
    }

    #[test]
    fn a_new_entry_starts_where_the_user_is_looking() {
        let mut entries = synthetic::scan_result(3).entries;
        entries[0].category = Category::ExtAssoc(".png".into());
        entries[1].category = Category::ProgId {
            prog_id: "pngfile".into(),
            from_ext: ".png".into(),
        };
        entries[2].category = Category::Drive;

        // The clicked row wins: it is the most specific thing on screen.
        assert_eq!(
            category_for_new_entry(Some(&entries[0]), Tab::Categories, None, None),
            Category::ExtAssoc(".png".into())
        );

        // A ProgID cannot be written to, and it is shared by several
        // extensions — so the entry goes under the one it was reached from.
        assert_eq!(
            category_for_new_entry(Some(&entries[1]), Tab::FileTypes, None, None),
            Category::ExtAssoc(".png".into())
        );

        // A base category comes through unchanged.
        assert_eq!(
            category_for_new_entry(Some(&entries[2]), Tab::Categories, None, None),
            Category::Drive
        );

        // Nothing clicked, but an extension chosen in the tree.
        assert_eq!(
            category_for_new_entry(None, Tab::FileTypes, Some(".jpg"), None),
            Category::ExtAssoc(".jpg".into())
        );

        // The extension only counts on the tab it belongs to.
        assert_eq!(
            category_for_new_entry(None, Tab::Categories, Some(".jpg"), Some(&Category::Folder)),
            Category::Folder
        );

        // Nothing at all: the most common case, and one click to change.
        assert_eq!(
            category_for_new_entry(None, Tab::Programs, None, None),
            Category::Directory
        );
    }

    #[test]
    fn every_preselected_category_can_actually_be_created() {
        // A preselection the writer then refuses would be worse than none.
        let exe = std::path::Path::new("x.exe");
        let _ = exe;

        for category in [
            Category::ExtAssoc(".png".into()),
            Category::PerceivedType("image".into()),
            Category::ProgId {
                prog_id: "pngfile".into(),
                from_ext: ".png".into(),
            },
            Category::ExtDirect(".rar".into()),
        ] {
            let chosen = creatable_category(&category).expect("has a creatable equivalent");
            let entry = crate::registry::create::NewEntry {
                category: chosen.clone(),
                key_name: "ctxmenu_x".into(),
                display_name: "x".into(),
                command: "x".into(),
                icon: None,
                position: None,
                extended: false,
                children: Vec::new(),
            };
            assert!(
                entry.target().is_ok(),
                "{category:?} maps to {chosen:?}, which cannot be written"
            );
        }
    }

    #[test]
    fn the_title_names_the_program_and_the_build() {
        // A screenshot in a bug report should say which build it came from,
        // and the name still has to follow the language setting.
        for tr in [&i18n::DE, &i18n::EN] {
            let title = window_title(tr);
            assert!(title.starts_with(tr.app_title), "{title}");
            assert!(title.ends_with(crate::VERSION), "{title}");
        }
        assert_ne!(
            window_title(&i18n::DE),
            window_title(&i18n::EN),
            "the name is translated even though the number is not"
        );
    }

    #[test]
    fn every_glyph_the_window_draws_is_in_a_font_it_loads() {
        // The failure this catches is silent: a character no loaded font
        // carries is drawn as an empty box, and nothing in the code, the tests
        // or the log says a word about it. Eight of them were on screen for
        // weeks — `↳ ⇧ ▴ ▸ ▾ ✓ ✗ ✕` — and only the two on buttons were ever
        // reported. The fonts are asked here rather than guessed at: this
        // reads the same character-to-glyph table the renderer looks in.
        use read_fonts::{FontRef, TableProvider};

        // What `install_fonts` puts in front, plus everything egui ships and
        // never removes. Taken from `FontDefinitions` rather than from a path,
        // so the two lists cannot drift apart.
        let mut faces: Vec<Vec<u8>> = egui::FontDefinitions::default()
            .font_data
            .values()
            .map(|data| data.font.to_vec())
            .collect();

        let mut own = 0;
        for path in [
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\SegUIVar.ttf",
            r"C:\Windows\Fonts\seguisym.ttf",
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                faces.push(bytes);
                own += 1;
            }
        }
        if own == 0 {
            // Not a Windows install with the system fonts in place; a verdict
            // from here would be about the machine, not about the code.
            return;
        }

        let covered = |ch: char| {
            ch.is_ascii()
                || faces.iter().any(|bytes| {
                    FontRef::new(bytes)
                        .ok()
                        .and_then(|font| font.cmap().ok())
                        .and_then(|cmap| cmap.map_codepoint(ch))
                        .is_some()
                })
        };

        // The check has to be able to say no, or it says nothing at all.
        // U+FF0B is the fullwidth plus these buttons wore until 2026-08-15,
        // and not one loaded font has it — Segoe UI Symbol included.
        assert!(
            !covered('\u{ff0b}'),
            "the font check never refuses anything, so it proves nothing"
        );

        let mut missing: Vec<String> = Vec::new();
        for (field, de, en) in i18n::field_pairs() {
            for text in [de, en] {
                for ch in text.chars() {
                    if !covered(ch) {
                        missing.push(format!("{field}: U+{:04X} {ch}", ch as u32));
                    }
                }
            }
        }
        for ch in UI_GLYPHS.chars() {
            if !covered(ch) {
                missing.push(format!("UI_GLYPHS: U+{:04X} {ch}", ch as u32));
            }
        }

        assert!(
            missing.is_empty(),
            "these would be drawn as empty boxes: {missing:#?}"
        );
    }

    #[test]
    fn nothing_the_window_says_is_said_twice() {
        // The window knows which language it is set to; a message that carries
        // both halves makes every reader read one they did not ask for. The
        // core keeps its bilingual strings for the console, which has no
        // setting to follow — that split is the entire point of the two
        // `Fault` types, and this test is what keeps them apart.
        let fav = [
            favourites::Fault::MissingName,
            favourites::Fault::MissingPath,
            favourites::Fault::FileNotFound(r"C:\Programme\tool.exe".into()),
            favourites::Fault::MissingAddress,
            favourites::Fault::InsecureAddress,
            favourites::Fault::NotHttps,
            favourites::Fault::NoPlaceholder,
        ];
        let entry = [
            Fault::MissingKeyName,
            Fault::BackslashInKeyName,
            Fault::MissingDisplayName,
            Fault::MissingCommand,
            Fault::PercentOneInBackground,
            Fault::AmpersandInDisplayName,
            Fault::UnusualPosition("Last".into()),
            Fault::CommandBesideSubmenu,
            Fault::ChildMissingDisplayName(2),
            Fault::ChildMissingCommand(2),
            Fault::DuplicateChildKeyName("01_x".into()),
            Fault::CategoryNotCreatable,
            Fault::UnusableKeyName,
        ];

        for tr in [&i18n::DE, &i18n::EN] {
            for fault in &fav {
                let text = fav_fault_text(fault, tr);
                assert!(!text.is_empty(), "{fault:?} has no wording");
                assert!(!text.contains(" / "), "{fault:?} says it twice: {text}");
            }
            for fault in &entry {
                let text = fault_text(fault, tr);
                assert!(!text.is_empty(), "{fault:?} has no wording");
                assert!(!text.contains(" / "), "{fault:?} says it twice: {text}");
            }
        }

        // And the other direction, because the console depends on it: there,
        // both halves have to be present.
        for fault in &fav {
            assert!(
                fault.bilingual().contains(" / "),
                "{fault:?} lost a language the console needs"
            );
        }
        for fault in &entry {
            assert!(fault.bilingual().contains(" / "), "{fault:?}");
        }
    }

    #[test]
    fn sorting_ignores_capitalisation() {
        // A list where WinRAR sorts before attrib because of a capital letter
        // is a list nobody can find anything in.
        let mut entries = synthetic::scan_result(3).entries;
        entries[0].display_name = "WinRAR".into();
        entries[1].display_name = "attrib".into();
        entries[2].display_name = "Zip".into();

        let mut order: Vec<&str> = entries.iter().map(|e| e.display_name.as_str()).collect();
        order.sort_by_key(|name| {
            entries
                .iter()
                .find(|e| e.display_name == *name)
                .map(|e| sort_key(e, SortBy::Name, &i18n::DE))
                .unwrap_or_default()
        });

        assert_eq!(order, ["attrib", "WinRAR", "Zip"]);
    }

    #[test]
    fn rows_with_nothing_to_say_sort_last() {
        // The scope column is empty for the base categories. Those rows are
        // not what somebody ordering by that column is looking for, so they
        // belong at the end rather than in front of the answer.
        let mut entries = synthetic::scan_result(2).entries;
        entries[0].category = Category::Directory;
        entries[1].category = Category::ExtAssoc(".zip".into());

        assert!(
            sort_key(&entries[1], SortBy::AppliesTo, &i18n::DE)
                < sort_key(&entries[0], SortBy::AppliesTo, &i18n::DE)
        );
    }

    #[test]
    fn the_location_is_said_in_words() {
        let mut entries = synthetic::scan_result(2).entries;
        entries[0].category = Category::AllFiles;
        entries[1].category = Category::ExtAssoc(".zip".into());

        // `*` means nothing to anyone who has not read the documentation.
        assert_eq!(appears_on(&entries[0], &i18n::DE), "Alle Dateien");
        assert_eq!(appears_on(&entries[0], &i18n::EN), "All Files");
        // A file type says itself, in both languages.
        assert_eq!(appears_on(&entries[1], &i18n::DE), ".zip");
        assert_eq!(appears_on(&entries[1], &i18n::EN), ".zip");
    }

    #[test]
    fn every_row_of_a_submenu_can_be_resolved_and_labelled() {
        // Guards the table's row closure: it resolves a row and then reads
        // four fields off it. A path that no longer fits must not panic.
        let parent = cascading();
        let mut rows = Vec::new();
        push_with_children(&mut rows, &parent, Row::top(0));

        let mut scan = synthetic::scan_result(1);
        scan.entries[0] = parent;

        for row in &rows {
            let entry = resolve(&scan, row).expect("row fits");
            assert!(!appears_on(entry, &i18n::DE).is_empty());
            assert!(!sort_key(entry, SortBy::Name, &i18n::DE).is_empty());
        }
    }

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
        assert_eq!(rows.iter().filter(|r| r.path.is_empty()).count(), 1);
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

    /// Builds a one-operation plan aimed at a given entry.
    fn plan_against(entry: &ContextEntry) -> Plan {
        let target = crate::registry::paths::RegTarget::parse(&entry.registry_path)
            .expect("a scanned path must parse back");
        Plan::new(
            "test",
            vec![Operation {
                target,
                action: Action::Delete,
                clsid: None,
                display_name: entry.display_name.clone(),
            }],
        )
    }

    #[test]
    fn a_submenu_child_can_be_named_as_a_target() {
        // The claim the old comment on `Row` got wrong. A child sits at
        // `…\shell\<parent>\shell\<child>`, and that names exactly one entry.
        let child = r"HKCU\SOFTWARE\Classes\Directory\shell\Attributes\shell\aShow";
        let target = crate::registry::paths::RegTarget::parse(child)
            .expect("a submenu child is a single entry");
        assert_eq!(target.full_path(), child);

        // Its parent's collection key still is not, which is the distinction
        // that has to survive: deleting that would take every sibling with it.
        assert!(
            crate::registry::paths::RegTarget::parse(
                r"HKCU\SOFTWARE\Classes\Directory\shell\Attributes\shell"
            )
            .is_err()
        );
    }

    #[test]
    fn a_child_is_dropped_when_its_parent_is_selected_too() {
        // Select-all now takes children as well, so this combination is one
        // click away rather than a curiosity.
        let parent = Row::top(7);
        let child = Row {
            entry: 7,
            path: vec![1],
        };
        let grandchild = Row {
            entry: 7,
            path: vec![1, 0],
        };

        assert!(parent.is_ancestor_of(&child));
        assert!(parent.is_ancestor_of(&grandchild));
        assert!(child.is_ancestor_of(&grandchild));

        // Not its own ancestor, not its sibling's, not another entry's.
        assert!(!parent.is_ancestor_of(&parent));
        assert!(!child.is_ancestor_of(&parent));
        assert!(
            !child.is_ancestor_of(&Row {
                entry: 7,
                path: vec![2, 0]
            }),
            "a sibling's subtree is not below this child"
        );
        assert!(
            !Row::top(7).is_ancestor_of(&Row {
                entry: 8,
                path: vec![0]
            }),
            "rows of different entries are unrelated"
        );
    }

    #[test]
    fn the_selection_holds_children_as_well_as_entries() {
        // A row set, not an index set: the second and third rows below differ
        // only in their path into `sub_commands`, and an index-based selection
        // could not tell them apart at all.
        let mut selection: rustc_hash::FxHashSet<Row> = Default::default();
        let parent = Row::top(7);
        let first_child = Row {
            entry: 7,
            path: vec![0],
        };
        let second_child = Row {
            entry: 7,
            path: vec![1],
        };

        selection.insert(parent.clone());
        selection.insert(first_child.clone());
        selection.insert(second_child.clone());
        assert_eq!(selection.len(), 3, "three distinct rows of one entry");

        selection.remove(&first_child);
        assert!(selection.contains(&parent), "the parent stays");
        assert!(selection.contains(&second_child), "the sibling stays");
        assert!(!selection.contains(&first_child));
    }

    #[test]
    fn deleting_an_entry_that_applies_to_every_file_says_how_far_it_reaches() {
        let mut entries = synthetic::scan_result(4).entries;
        entries[0].category = Category::AllFiles;
        entries[1].category = Category::AllFilesystemObjects;
        entries[2].category = Category::ExtAssoc(".zip".into());

        // Level 1 and level 2 both apply to every file, so both warn.
        assert_eq!(
            breadth_of_plan(&plan_against(&entries[0]), &entries, 98),
            Some(98)
        );
        assert_eq!(
            breadth_of_plan(&plan_against(&entries[1]), &entries, 98),
            Some(98)
        );

        // An entry that really does belong to one type says nothing: a
        // warning on every dialog is a warning nobody reads.
        assert_eq!(
            breadth_of_plan(&plan_against(&entries[2]), &entries, 98),
            None
        );
    }

    #[test]
    fn without_a_file_type_scan_there_is_no_number_to_show() {
        // The command line scans base categories only. Naming a count of zero
        // types, or inventing one, would both be worse than staying quiet.
        let mut entries = synthetic::scan_result(2).entries;
        entries[0].category = Category::AllFiles;
        assert_eq!(
            breadth_of_plan(&plan_against(&entries[0]), &entries, 0),
            None
        );
    }

    #[test]
    fn the_warning_follows_the_plan_not_the_selection() {
        // An entry that was selected but did not make it into the plan must
        // not produce a warning about something that will not happen.
        let mut entries = synthetic::scan_result(3).entries;
        entries[0].category = Category::AllFiles;
        entries[1].category = Category::Directory;

        let plan = plan_against(&entries[1]);
        assert_eq!(breadth_of_plan(&plan, &entries, 98), None);
    }

    #[test]
    fn registry_paths_are_matched_case_insensitively() {
        let mut entries = synthetic::scan_result(2).entries;
        entries[0].category = Category::AllFiles;
        let plan = plan_against(&entries[0]);
        entries[0].registry_path = entries[0].registry_path.to_uppercase();

        assert_eq!(breadth_of_plan(&plan, &entries, 98), Some(98));
    }

    #[test]
    fn a_list_cursor_clamps_at_both_ends() {
        assert_eq!(next_cursor(Some(0), 3, Movement::Up), Some(0));
        assert_eq!(next_cursor(Some(2), 3, Movement::Down), Some(2));
        assert_eq!(next_cursor(Some(1), 3, Movement::Down), Some(2));
        assert_eq!(next_cursor(Some(1), 3, Movement::Up), Some(0));
    }

    #[test]
    fn every_key_lands_somewhere_when_nothing_was_selected() {
        // Pressing a key and having nothing happen reads as a broken list.
        for movement in [Movement::Down, Movement::Up, Movement::First] {
            assert_eq!(next_cursor(None, 4, movement), Some(0), "{movement:?}");
        }
        assert_eq!(next_cursor(None, 4, Movement::Last), Some(3));
    }

    #[test]
    fn home_and_end_ignore_where_the_cursor_was() {
        assert_eq!(next_cursor(Some(2), 5, Movement::First), Some(0));
        assert_eq!(next_cursor(Some(2), 5, Movement::Last), Some(4));
    }

    #[test]
    fn an_empty_list_has_no_cursor_at_all() {
        // The index goes straight into `favourites[..]`, so "no rows" must not
        // produce a zero.
        for movement in [
            Movement::Down,
            Movement::Up,
            Movement::First,
            Movement::Last,
        ] {
            assert_eq!(next_cursor(None, 0, movement), None, "{movement:?}");
            assert_eq!(next_cursor(Some(3), 0, movement), None, "{movement:?}");
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
