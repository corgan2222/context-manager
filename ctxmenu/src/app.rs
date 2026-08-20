//! The application window.
//!
//! Immediate mode means this whole file runs many times per second, so one
//! rule governs everything here: no registry access, no icon extraction, no
//! version resource lookup in the frame path. Anything costly is precomputed
//! and kept in the state, and anything slow runs on a thread.

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
use crate::service::{self, Service, grouping, spec};
use crate::settings::{Language, Settings, ThemeChoice};
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Categories,
    FileTypes,
    Programs,
    Favourites,
    Services,
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
            "services" | "dienste" => Some(Tab::Services),
            "backups" | "sicherungen" => Some(Tab::Backups),
            _ => None,
        }
    }
}

/// What the command line asked the window to start as.
///
/// One value rather than a parameter each. `run` and `App::new` both need the
/// whole set, and passing them one by one had already reached the seven
/// arguments clippy allows before it calls a signature unreadable — which it
/// is: the second `Option<usize>` in a row is a slip away from being the
/// wrong one, and the compiler would not notice.
///
/// [`Default`] is the plain `ctxmenu` with no arguments at all: scan the
/// registry, open on the first tab, in the saved language, at the usual size.
#[derive(Debug, Clone, Default)]
pub struct Start {
    /// Fill the table with generated rows instead of scanning, for the
    /// performance target of milestone 4.
    pub synthetic: Option<usize>,
    /// Run this many measured frames, report and exit.
    pub bench: Option<usize>,
    /// Which tab to open on.
    pub tab: Tab,
    /// Text to put in the search box before the first frame. Exists so a
    /// search can be photographed and checked, not only tried by hand.
    pub search: String,
    /// Extension to preselect in the file type tab. It exists so that tab can
    /// be measured at all: without a selection it shows nothing, and its one
    /// real fault was invisible from outside.
    pub ext: Option<String>,
    /// The service to select and load, by the `id` it carries in
    /// `services.json`. The same reason as `ext`, on the tab that needed it
    /// most: without a selection the services tab shows one name on the left
    /// and a sentence asking for a click on the right, which is a picture of
    /// nothing. The id and not a position in the list, because a position
    /// moves as soon as a service is added and an id never does.
    pub service: Option<String>,
    /// Open the editor on a new entry in this category, filled in with
    /// [`create::example_entry`]. The one form this program has offered itself
    /// to a right-click and to nothing else, so it could not be photographed
    /// at all. Opening it writes nothing: the button inside it still has to be
    /// pressed.
    pub new_entry: Option<Category>,
    /// Flip the system theme once while running and report whether the window
    /// followed. Restores the setting afterwards.
    pub theme_probe: bool,
    /// The language for this run only. `None` keeps the saved choice, which is
    /// what every run that nobody said otherwise about gets. Setting it does
    /// not write the settings file — a screenshot in the other language must
    /// not cost the user their preference.
    pub language: Option<Language>,
    /// The window size for this run only, in physical pixels. `None` opens at
    /// the size this window has always opened at.
    ///
    /// Physical rather than logical points, because a size is asked for in
    /// order to photograph it and a photograph is measured in pixels. A run
    /// that names a size also places itself on the leftmost screen, the way
    /// every other automatic run does — see `App::place_window_once`.
    pub size: Option<(i32, i32)>,
}

/// What the worker sends back after fetching a description: the address that
/// answered, and the tools read out of it — or why neither happened.
type FetchedSpec = Result<(String, Vec<spec::Tool>), String>;

/// Placeholder shown in the empty command field.
const HINT_COMMAND: &str = "\"C:\\Windows\\notepad.exe\" \"%1\"";
/// Placeholder shown in the empty icon field.
const HINT_ICON: &str = "C:\\Windows\\notepad.exe,0";
/// Placeholders in the favourite editor.
const HINT_PROGRAM: &str = "C:\\Program Files\\Werkzeug\\werkzeug.exe";
const HINT_ARGS: &str = "--flag \"%1\"";
const HINT_URL: &str = "https://squoosh.app";
const HINT_ENDPOINT: &str = "https://api.tinify.com/shrink";
/// Placeholders in the service form.
const HINT_SERVICE_NAME: &str = "SnapOtter";
const HINT_SPEC_URL: &str = "http://192.168.2.11:1349/api/docs/";
const HINT_KEY: &str = "Bearer si_...";
const HINT_RESULT_PATH: &str = "downloadUrl";
const HINT_SETTINGS: &str = "{\"width\": 1920}";
/// What the result of a service tool is called next to the original.
///
/// One suffix for every tool of every service: the alternative is asking a
/// hundred times, and the tool's own name is already in the menu entry.
const SERVICE_SUFFIX: &str = ".neu";

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
        /// What the form was filled in from, or `None` when it is a new one —
        /// which is also how insert and replace are told apart.
        ///
        /// Kept beside the draft because a *second* process writes this file
        /// while the form stands open: the one a right-click on a web tool
        /// starts, which records the user's agreement to sending files. The
        /// two copies together say which fields this form actually changed,
        /// and only those are the form's to write. See
        /// [`crate::favourites::update`].
        before: Option<Box<Favourite>>,
    },
    /// The form for one service — an address, a key, and how it answers.
    Service {
        draft: Box<Service>,
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
        /// and the frame path has no business touching one.
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
    Created {
        path: String,
        /// What went wrong beside the entry — today only a record in
        /// `entries.json` that could not be written. Marked bilingual and cut
        /// where it is drawn, like `Error`, so switching the language while
        /// the window stands open redraws it.
        note: Option<String>,
    },
}

/// How far the self-update has got.
///
/// Nothing here opens a window of its own. The check runs on start, may well
/// fail because a machine is offline, and a program that greets its user with
/// "could not reach GitHub" for that is a program people turn off. So the whole
/// state lives in the About window, and the only thing that leaks out of it is
/// a dot on the button that opens it.
#[derive(Debug, Default)]
enum UpdateState {
    /// Nobody has asked yet, or the setting says not to.
    #[default]
    Unknown,
    Checking,
    /// Asked, and this is the newest there is.
    Current,
    /// There is a newer one, and it carries everything needed to install it.
    Available(Box<crate::update::Available>),
    /// There is a newer one, and its assets are not all there yet. Nothing to
    /// press; the version is named and the sentence says to look again.
    Incomplete(String),
    Downloading,
    /// Installed. The new copy is starting and this window is closing.
    Restarting,
    /// Kept bilingual and cut where it is drawn, like `Dialog::Error`, so
    /// switching the language redraws it.
    Failed(String),
}

impl UpdateState {
    /// Whether a worker is busy, which is what greys out both buttons.
    fn busy(&self) -> bool {
        matches!(
            self,
            UpdateState::Checking | UpdateState::Downloading | UpdateState::Restarting
        )
    }
}

/// Release notes as something worth reading in a dialog box.
///
/// GitHub hands them over as Markdown, and `release-drafter` starts every set
/// with a `## What's Changed` heading. This window has no Markdown renderer and
/// would be a strange place to grow one, so the hashes come off and the rest
/// stands as it is: a list of `*` lines reads perfectly well without them, and
/// the heading is already above the box in the user's own language.
///
/// Runs of blank lines collapse to one. Markdown uses them for paragraph
/// breaks and a changelog often carries two or three in a row, which in a box
/// 140 points high is most of the box.
fn plain_notes(notes: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in notes.lines() {
        let line = line.trim_end().trim_start_matches(['#', ' ']);
        if line.is_empty() && lines.last().is_some_and(|last: &&str| last.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_string()
}

/// What a click in the update part of the About window asked for.
///
/// Ticking the box is not in here: whether the setting changed is a comparison
/// against what it was, and one way of finding that out is enough.
enum UpdateAction {
    Check,
    Install,
}

/// What a worker sends back.
enum UpdateMessage {
    Checked(Result<crate::update::Outcome, String>),
    /// Where the new executable now is.
    Installed(Result<std::path::PathBuf, String>),
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

/// A worked example for the upload form, from a real service.
///
/// Written out rather than invented: these four lines are what made a
/// self-hosted tool service work on 2026-08-15, and every one of them is a
/// field somebody would otherwise have to guess.
const UPLOAD_EXAMPLE_ENDPOINT: &str = "http://192.168.2.11:1349/api/v1/tools/image/compress";
const UPLOAD_EXAMPLE_FIELD: &str = "file";
const UPLOAD_EXAMPLE_HEADER: &str = "Authorization: Bearer si_4e8a0c…";
const UPLOAD_EXAMPLE_PATH: &str = "downloadUrl";

/// The Feather glyphs this window draws, looked up once at startup.
///
/// `try_icon` searches a generated table of some three hundred names. Doing that
/// per button per frame is exactly the work that has no place in the frame
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
    restore: char,
    explorer: char,
    link: char,
    /// The three faces of the theme button: a screen, a sun, a moon.
    theme_system: char,
    theme_light: char,
    theme_dark: char,
}

impl Glyphs {
    /// The names, in one place, so the test below can walk the same list.
    const NAMES: [&'static str; 23] = [
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
        "rotate-ccw",
        "folder",
        "external-link",
        "monitor",
        "sun",
        "moon",
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
            restore: next(),
            explorer: next(),
            link: next(),
            theme_system: next(),
            theme_light: next(),
            theme_dark: next(),
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

/// A "new entry here" line, for every place a right-click can land.
///
/// The same offer from the category tree, the file type list, the program list
/// and the empty space below the table — each of them knowing which category
/// "here" means. Returns the category to create in when it was clicked.
fn new_entry_menu(
    ui: &mut Ui,
    glyphs: &Glyphs,
    tr: &'static Strings,
    category: Category,
) -> Option<Category> {
    let mut chosen = None;
    if ui
        .button(labelled(glyphs.new, tr.ctx_new_entry))
        .on_hover_text(tr.tip_editor_new)
        .clicked()
    {
        chosen = Some(category);
        ui.close();
    }
    chosen
}

/// The file just dropped on this rectangle, if one was.
///
/// For the fields inside a dialog, where the drop means "put this path here"
/// rather than "make an entry out of it". The check is against the rectangle
/// and not against `hovered`, because a drag coming from Explorer never gives
/// egui a hover — the pointer belongs to the drag, not to the window.
fn dropped_on(ui: &Ui, rect: egui::Rect) -> Option<std::path::PathBuf> {
    if !ui.rect_contains_pointer(rect) {
        return None;
    }
    ui.ctx().input(|input| {
        input
            .raw
            .dropped_files
            .first()
            .map(|file| file.path().to_path_buf())
    })
}

/// Is a file being dragged over the window, or landing right now?
///
/// Both, because the frame that reports the drop no longer reports the hover:
/// a drop target noted only while `hovered_files` is filled would already be
/// forgotten when `dropped_files` arrives.
fn files_in_the_air(ctx: &egui::Context) -> bool {
    ctx.input(|input| !input.raw.hovered_files.is_empty() || !input.raw.dropped_files.is_empty())
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
    /// How many are entries of the new Windows 11 menu. They know neither
    /// the Shift rule nor a position, and deleting means uninstalling the
    /// package — three switch groups go grey when this is non-zero.
    packaged: usize,
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
    // A packaged entry has no Extended flag, no position and no key to
    // delete — its menu offers exactly what the plan path can do for it.
    if let EntryKind::PackagedVerb {
        blocked_machine, ..
    } = &entry.kind
    {
        out.push(match blocked_machine {
            true => Action::Unblock,
            false => Action::Block,
        });
        return out;
    }
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

/// The face the theme button wears for each of its three states.
///
/// A screen for "whatever Windows says", a sun for light, a moon for dark. The
/// picture is the state, which is the whole point of the button: the drop-down
/// it replaced spent a hundred points on saying the same thing in words.
fn theme_glyph(choice: ThemeChoice, glyphs: &Glyphs) -> char {
    match choice {
        ThemeChoice::System => glyphs.theme_system,
        ThemeChoice::Light => glyphs.theme_light,
        ThemeChoice::Dark => glyphs.theme_dark,
    }
}

/// What the theme button says when the pointer rests on it.
///
/// Every state gets a whole sentence naming both where it stands and where the
/// next click goes. A symbol nobody has seen before is a guess until it says so
/// itself.
fn theme_tip(choice: ThemeChoice, tr: &'static Strings) -> &'static str {
    match choice {
        ThemeChoice::System => tr.tip_theme_system,
        ThemeChoice::Light => tr.tip_theme_light,
        ThemeChoice::Dark => tr.tip_theme_dark,
    }
}

/// The proportions of the two flags the language button draws, width to height.
///
/// Both are drawn at the same size rather than at their official ratios (5:3
/// for Germany, 2:1 for the United Kingdom): they sit in the same button and
/// swap places on a click, and a button that changed width as it was pressed
/// would push the rest of the toolbar sideways under the pointer.
const FLAG_ASPECT: f32 = 1.5;

/// The three bands of the German flag, top to bottom: black, red, gold.
///
/// Derived from the rectangle rather than from fixed numbers, so the same
/// function serves the toolbar at any font size or screen scaling. The bands
/// are cut at rounded pixel boundaries because a band whose edge lands on half
/// a pixel comes out as a grey seam between two colours.
fn german_bands(rect: egui::Rect) -> [egui::Rect; 3] {
    let cut = |part: f32| rect.top() + (rect.height() * part / 3.0).round();
    let (first, second) = (cut(1.0), cut(2.0));
    [
        egui::Rect::from_x_y_ranges(rect.x_range(), rect.top()..=first),
        egui::Rect::from_x_y_ranges(rect.x_range(), first..=second),
        egui::Rect::from_x_y_ranges(rect.x_range(), second..=rect.bottom()),
    ]
}

/// The upright and the crossbar of the Union Jack's Saint George's cross.
///
/// `width` is the thickness of the bar; both are centred, and both are clamped
/// to the rectangle so a thickness larger than the flag cannot paint outside
/// it. The saltire underneath is drawn as two clipped line segments instead —
/// a diagonal has no rectangle to describe it.
fn union_bars(rect: egui::Rect, width: f32) -> [egui::Rect; 2] {
    let half = (width / 2.0)
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0);
    let centre = rect.center();
    [
        egui::Rect::from_x_y_ranges(centre.x - half..=centre.x + half, rect.y_range()),
        egui::Rect::from_x_y_ranges(rect.x_range(), centre.y - half..=centre.y + half),
    ]
}

/// The two diagonals of the Union Jack, corner to corner.
///
/// Corner to corner and nothing else: the real flag counterchanges the red arms
/// of the saltire against the white ones, and at the size the toolbar gives it —
/// measured on this machine at 21 by 14 points, 32 by 21 screen pixels at 150 %
/// scaling — that offset is under a pixel.
fn union_diagonals(rect: egui::Rect) -> [[egui::Pos2; 2]; 2] {
    [
        [rect.left_top(), rect.right_bottom()],
        [rect.left_bottom(), rect.right_top()],
    ]
}

/// Draws the German flag into `rect`.
fn paint_german_flag(painter: &egui::Painter, rect: egui::Rect) {
    const BLACK: egui::Color32 = egui::Color32::from_rgb(0x00, 0x00, 0x00);
    const RED: egui::Color32 = egui::Color32::from_rgb(0xDD, 0x00, 0x00);
    const GOLD: egui::Color32 = egui::Color32::from_rgb(0xFF, 0xCE, 0x00);

    for (band, colour) in german_bands(rect).into_iter().zip([BLACK, RED, GOLD]) {
        painter.rect_filled(band, 0.0, colour);
    }
}

/// Draws the flag of the United Kingdom into `rect`.
///
/// Painted in the order the flag was built: the blue field, the white saltire,
/// the red saltire on top of it, then the white cross of Saint George and its
/// red centre. The painter handed in must already be clipped to `rect`, because
/// the diagonals are drawn as thick strokes through the corners and a stroke
/// has width in both directions.
fn paint_union_jack(painter: &egui::Painter, rect: egui::Rect) {
    const BLUE: egui::Color32 = egui::Color32::from_rgb(0x01, 0x21, 0x69);
    const RED: egui::Color32 = egui::Color32::from_rgb(0xC8, 0x10, 0x2E);
    const WHITE: egui::Color32 = egui::Color32::WHITE;

    // Thicknesses as fractions of the height, close to the official ones
    // (the cross is a fifth of the hoist) and rounded up to a full pixel so
    // nothing ends up as a grey smear at this size.
    let cross = (rect.height() / 5.0).max(2.0);
    let saltire = (rect.height() / 7.0).max(1.5);

    painter.rect_filled(rect, 0.0, BLUE);
    for [from, to] in union_diagonals(rect) {
        painter.line_segment([from, to], egui::Stroke::new(saltire, WHITE));
    }
    for [from, to] in union_diagonals(rect) {
        painter.line_segment([from, to], egui::Stroke::new(saltire / 2.5, RED));
    }
    for bar in union_bars(rect, cross) {
        painter.rect_filled(bar, 0.0, WHITE);
    }
    for bar in union_bars(rect, cross * 0.55) {
        painter.rect_filled(bar, 0.0, RED);
    }
}

/// The language button: the flag of the language in force, as a button.
///
/// A picture where two words used to be. The flag is framed in a thin line of
/// the button's own text colour, because both flags carry white — without an
/// edge the Union Jack's white arms would bleed into a light theme's toolbar.
///
/// The height is handed in rather than worked out here, and what the toolbar
/// hands in is the height of the button beside it: egui derives a text button's
/// height from the font and the button style, and a picture button that guessed
/// at the same number would stand a point or two taller than its neighbour —
/// which in a row of two buttons is exactly where it shows.
fn flag_button(ui: &mut Ui, language: Language, height: f32) -> egui::Response {
    let padding = ui.spacing().button_padding;
    let flag_height = (height - 2.0 * padding.y).max(6.0).round();
    let flag_size = egui::vec2((flag_height * FLAG_ASPECT).round(), flag_height);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(flag_size.x + 2.0 * padding.x, height),
        egui::Sense::click(),
    );

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );

        let flag = egui::Rect::from_center_size(rect.center(), flag_size);
        // Clipped, so the saltire's thick strokes stop at the flag's edge
        // instead of reaching into the button's frame.
        let inside = ui.painter().with_clip_rect(flag);
        match language {
            Language::German => paint_german_flag(&inside, flag),
            Language::English => paint_union_jack(&inside, flag),
        }
        ui.painter().rect_stroke(
            flag,
            0.0,
            egui::Stroke::new(1.0, visuals.fg_stroke.color),
            egui::StrokeKind::Inside,
        );
    }

    response
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
        packaged: 0,
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
    state.packaged = entries
        .iter()
        .filter(|entry| matches!(entry.kind, EntryKind::PackagedVerb { .. }))
        .count();

    state.hidden = agreement(entries.iter().map(|entry| entry.hidden));
    state.extended = agreement(entries.iter().map(|entry| entry.extended));
    state.position = agreement(entries.iter().map(|entry| entry.position.clone()));
    state.blocked = agreement(entries.iter().filter_map(|entry| match &entry.kind {
        EntryKind::ShellEx { clsid, blocked, .. } if !clsid.is_empty() => Some(*blocked),
        EntryKind::PackagedVerb {
            blocked_machine, ..
        } => Some(*blocked_machine),
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
        EntryKind::PackagedVerb { clsid, .. } => Some(clsid),
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
    /// sorting are evaluated here once, not per frame.
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
    /// Whether this Windows has the new context menu at all, and whether the
    /// classic one is currently switched on.
    ///
    /// Both read once at startup: the build number never changes while the
    /// program runs, and the switch only changes through this window.
    win11: bool,
    classic_menu: bool,
    /// Whether the handler package that serves own entries to the new menu
    /// is registered. Read at startup and after each install or remove —
    /// never in the frame path, it is a registry enumeration.
    handler_installed: bool,
    /// The last error already written to the log.
    ///
    /// An error dialog re-sets itself every frame it stays open, so without
    /// this the log would grow by sixty identical lines a second.
    logged_error: Option<String>,
    /// Which backup the detail pane is describing.
    ///
    /// The list used to hide everything behind a collapsing header, which meant
    /// the answer to "what is in this one" was a click away and then filled the
    /// list itself. It reads like every other tab now: pick on the left, read on
    /// the right.
    selected_backup: Option<std::path::PathBuf>,
    /// The category a hovering file would land in.
    ///
    /// A field and not a local, because the frame that reports the drop is not
    /// the frame that had the pointer over a row: `dropped_files` arrives once,
    /// and by then `hovered_files` is empty again and the tree has already been
    /// drawn. So the target is noted while files are in the air and read when
    /// one lands. That is also why the first version did nothing at all.
    drop_target: Option<Category>,
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

    /// The services a favourite can be made from (`services.json`).
    services: Vec<Service>,
    service_error: Option<String>,
    /// Index into `services` whose tools are on screen.
    service_focus: Option<usize>,
    /// The tools of the focused service, as its description listed them.
    ///
    /// Not persisted: a description is fetched again when it is wanted, and
    /// keeping a copy would mean showing tools the service no longer has.
    service_tools: Vec<spec::Tool>,
    /// How this service's tools fall into groups.
    ///
    /// Worked out once when the description arrives, never per frame and never
    /// from a filtered view: on the result of a search box the category segment
    /// is often constant, and some incidental axis would win instead.
    service_grouping: Option<grouping::Grouping>,
    /// Which tools are ticked, by index into `service_tools`.
    service_picked: rustc_hash::FxHashSet<usize>,
    /// What was typed into the settings field of a tool, by the same index.
    /// Used where the description only says "a string, and here is what may go
    /// in it" — the common case.
    service_settings: rustc_hash::FxHashMap<usize, String>,
    /// One entry per typed field of a tool that came with a real schema, keyed
    /// by tool index and field name. Everything is held as text and converted
    /// on saving, so a half-typed number is not a state this has to model.
    service_fields: rustc_hash::FxHashMap<(usize, String), String>,
    /// Which tool has its settings unfolded. One at a time: a hundred open
    /// forms is a wall of text, and only the ticked ones matter anyway.
    service_open: Option<usize>,
    /// Filters the tool list. With 351 endpoints the list is unusable without
    /// it, and it is the first thing anyone reaches for.
    service_search: String,
    /// Whether the tools that answer with a job number are listed at all.
    ///
    /// Off: they cannot be made into a working entry, and on the test service
    /// they are 52 of 232 — a fifth of the list that exists only to be greyed
    /// out. The count stays visible so nothing disappears silently.
    service_show_async: bool,
    /// Receives a fetched description: the address that answered and its tools.
    service_rx: Option<Receiver<FetchedSpec>>,

    /// How far the self-update has got. Drawn in the About window only.
    update: UpdateState,
    /// Receives what the update worker found or installed.
    update_rx: Option<Receiver<UpdateMessage>>,

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
    /// Whether the width of the two settings buttons has been reported yet.
    ///
    /// The toolbar is the one row that has to hold everything at once, so what
    /// a control costs there is a number worth having rather than guessing at.
    /// Reported once, from the frame that drew them, because that is the only
    /// place the real style and the real font are known.
    settings_width_reported: bool,
    /// Running only in the probe mode, and only for as long as it takes.
    theme_probe: Option<ThemeProbe>,
    /// Still owing the window a move to the leftmost screen.
    place_left: bool,
    /// How big to make it when that move happens, in physical pixels. `None`
    /// takes the full width of the screen, which is what an unattended run
    /// wants unless it said otherwise.
    place_size: Option<(i32, i32)>,
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

impl Bench {
    /// Counts one frame off `remaining`, without ever wrapping past zero.
    ///
    /// `cli::parse` already refuses `--bench 0`, since a run over zero
    /// frames measures nothing. This is the other half: the check sits
    /// *before* the subtraction, not after, so that this counter is safe on
    /// its own even so — for instance against one more frame still arriving
    /// after the finished run below already asked the window to close.
    /// Without it, `remaining -= 1` on an already-zero counter wrapped to
    /// `usize::MAX` in release (where overflow checks are off), and the
    /// benchmark never finished at all.
    ///
    /// Returns whether this frame was actually counted.
    fn count_down(&mut self) -> bool {
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        true
    }
}

impl App {
    /// Everything the command line had to say about this run arrives in
    /// [`Start`]; `Start::default()` is the plain double-click.
    pub fn new(cc: &eframe::CreationContext<'_>, start: Start) -> Self {
        install_fonts(&cc.egui_ctx);

        let mut settings = Settings::load_or_default(theme::system_language());
        // `--lang` moves this run and nothing else. Written into the loaded
        // settings rather than kept beside them so the whole window agrees on
        // one language — the box in the tool bar included — and the file stays
        // as it was: nothing here saves, and the only thing that does is the
        // user changing something, at which point saving what is on screen is
        // exactly right.
        if let Some(language) = start.language {
            settings.language = language;
        }
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

        crate::bilingual::set_language(settings.language);
        let tr = strings_for(settings.language);
        let hwnd = theme::window_handle(cc);

        // Unlike the favourites list below, a load failure here has to be
        // kept rather than swallowed: `--tab services` opens straight on the
        // services tab, so the tab-click handler that would otherwise call
        // `reload_services` and surface the error never runs. Losing the
        // message would leave a damaged `services.json` looking like an
        // empty, healthy one.
        let (services, service_error) = services_from_load(service::load());

        let mut app = Self {
            scan: None,
            visible_rows: Vec::new(),
            filter_dirty: true,
            sort: (SortBy::Natural, true),
            scroll_to_top: false,
            tab: start.tab,
            selected_category: None,
            selected_ext: start.ext,
            ext_draft: String::new(),
            scan_every_type: false,
            selected_group: None,
            groups: Vec::new(),
            selected: rustc_hash::FxHashSet::default(),
            focused: None,
            anchor: None,
            selected_backup: None,
            win11: crate::registry::win11::has_new_menu(),
            classic_menu: crate::registry::win11::classic_menu(),
            handler_installed: crate::registry::win11::has_new_menu()
                && crate::handler::is_installed(),
            logged_error: None,
            drop_target: None,
            search: start.search,
            dialog: None,
            action_rx: None,
            scan_rx: None,
            scanning: false,
            progress: (0, 0),
            progress_label: String::new(),
            backups: Vec::new(),
            backup_error: None,
            full_backup_rx: None,
            // Read here and not only when the tab is entered. Starting on the
            // favourites tab — which `--tab favourites` does, and which is
            // where the window reopens after using it — showed an empty tool
            // box and the sentence about there being nothing saved yet, with a
            // full favourites.json on disk. Once, in the constructor: a file
            // has no business in the frame path, and this one is
            // read again after every change anyway.
            favourites: favourites::load().unwrap_or_default(),
            favourite_error: None,
            favourite_focus: None,
            favourite_scroll: false,
            // Same reasoning as the favourites: a small file, read once. The
            // error, unlike theirs, is kept -- see above.
            services,
            service_error,
            service_focus: None,
            service_tools: Vec::new(),
            service_grouping: None,
            service_picked: rustc_hash::FxHashSet::default(),
            service_settings: rustc_hash::FxHashMap::default(),
            service_fields: rustc_hash::FxHashMap::default(),
            service_open: None,
            service_search: String::new(),
            service_show_async: false,
            service_rx: None,
            update: UpdateState::default(),
            update_rx: None,
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
            settings_width_reported: false,
            first_list_ms: None,
            bench: start.bench.map(|frames| Bench {
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
            // main one is the user's desk. A run that asked for a size joins
            // them — the size is asked for in order to photograph the window,
            // and a photograph wants it in a known place as much as at a
            // known size.
            place_left: start.theme_probe || start.bench.is_some() || start.size.is_some(),
            place_size: start.size,
            theme_probe: start.theme_probe.then(|| ThemeProbe {
                stage: ProbeStage::Settling {
                    left: PROBE_SETTLE_FRAMES,
                },
                frames: 0,
            }),
        };

        match start.synthetic {
            Some(count) => {
                app.scan = Some(crate::synthetic::scan_result(count));
                app.filter_dirty = true;
            }
            None => app.start_scan(&cc.egui_ctx),
        }

        app.reload_backups();

        // What the last update left behind, and the question whether there is
        // another one. Both on threads: one touches the disk, the other the
        // network, and the window is opening.
        std::thread::spawn(crate::update::clean_up);
        if app.settings.check_for_updates {
            app.start_update_check(&cc.egui_ctx);
        }

        // `--service <id>`: the same two steps a click on the name takes, so
        // the tab opens with a service selected and its tools on their way in.
        // An id nobody knows is reported and nothing else: the window stays
        // open, on the services tab, with the message where the fetch error
        // would be — and a run that cannot reach the service at all ends in
        // the same place, through `poll_services`.
        if let Some(id) = start.service.as_deref() {
            match service::index_of(&app.services, id) {
                Ok(index) => app.select_service(index, &cc.egui_ctx),
                Err(error) => {
                    let message = format!("{error:#}");
                    // On the console as well as in the window: this argument
                    // is used by scripts, and a script reads stderr rather
                    // than a panel.
                    crate::errln!("{message}");
                    app.service_error = Some(message);
                }
            }
        }

        // `--new <category>`: the editor, filled in with the example. Opening
        // a dialog is all it does; every path that writes runs from a button
        // inside the form.
        if let Some(category) = start.new_entry.clone() {
            let example = create::example_entry(category, app.settings.language);
            app.open_editor(example);
        }

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
        // business doing that.
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
                    // what a right-click on this type really offers — but they
                    // are also identical for every type, and for `.jpg` they
                    // are 39 rows against 19. Off by default since 2026-08-15,
                    // and one checkbox away.
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

            Tab::Backups | Tab::Favourites | Tab::Services => Vec::new(),
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
                    before: Some(Box::new(favourite.clone())),
                    draft: Box::new(favourite),
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
    /// test, or one that named a window size on the command line. A window
    /// that a person opened belongs wherever that person wants it, and
    /// dragging it away would be its own kind of rude.
    ///
    /// Not done through `ViewportBuilder`, because the handle does not exist
    /// before the window does; the first frame is the earliest moment this can
    /// happen at all. It is also the only way to get a size in pixels rather
    /// than in points: `ViewportBuilder` speaks logical points, and on this
    /// machine's 150 % screens the two differ by half again.
    fn place_window_once(&mut self) {
        if !self.place_left {
            return;
        }
        let Some(hwnd) = self.hwnd else { return };
        self.place_left = false;

        match theme::place_on_left_screen(hwnd, self.place_size) {
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

        if !bench.count_down() {
            // The benchmark already finished on an earlier frame and asked
            // the window to close (below); if egui still calls this once
            // more before it actually does, there is nothing left to count.
            return;
        }
        bench.scroll += 7;

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
                        _ => "no reaction: the RegNotifyChangeKeyValue fallback is due",
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
            // and no equivalent, which is why LegacyDisable is offered there
            // instead.
            if matches!(action, Action::Block | Action::Unblock) && clsid.is_none() {
                continue;
            }

            operations.push(Operation {
                target,
                action: action.clone(),
                clsid,
                display_name: entry.display_name.clone(),
                packaged: matches!(entry.kind, EntryKind::PackagedVerb { .. }),
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
    /// takes it away from all 98 types at once.
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
                // The wide kind: these are containers such as
                // `Directory\shell`, not the keys of one action, so a branch
                // this Windows never had stays noted rather than removed on a
                // restore months later.
                let result = backup::export_wide("gesamt", &paths)
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
                let first = crate::registry::plan::execute(&direct).map_err(|e| format!("{e:#}"));

                let outcome = match elevated.is_empty() {
                    true => first,
                    false => combine_halves(
                        first,
                        elevation::run_elevated(&elevated).map_err(|e| format!("{e:#}")),
                        tr,
                    ),
                };

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

/// Folds the two halves of a split action into one report.
///
/// A free function rather than four branches inside the worker thread, because
/// the case that was wrong here cannot be reached by hand without a UAC prompt
/// and two hives: the direct half failing — only a failed backup export does
/// that — while the elevated half succeeds. Its report used to be dropped on
/// the floor, although its changes were already on the machine and its backup
/// already on disk, and the user saw nothing but the other half's error.
///
/// Whichever half ran is reported, and the half that did not becomes a failed
/// row with its message. Only when both fail is there nothing to show, and
/// then both reasons are said rather than one.
fn combine_halves(
    direct: Result<Report, String>,
    elevated: Result<Report, String>,
    tr: &'static Strings,
) -> Result<Report, String> {
    let row = |name: &str, message: String| crate::registry::plan::OperationResult {
        display_name: name.to_string(),
        registry_path: String::new(),
        action: Action::Hide,
        error: Some(message),
    };

    match (direct, elevated) {
        (Ok(mut first), Ok(second)) => {
            first.merge(second);
            Ok(first)
        }
        // Partial success plus a declined prompt is not a failure of the whole
        // operation.
        (Ok(mut first), Err(message)) => {
            first.results.push(row(tr.elevated_part, message));
            Ok(first)
        }
        (Err(message), Ok(mut second)) => {
            second.results.insert(0, row(tr.direct_part, message));
            Ok(second)
        }
        (Err(first), Err(second)) => Err(format!("{first}\n{second}")),
    }
}

/// Puts several backups back and folds their reports into one.
///
/// A split action leaves two directories, and "undo what I just did" means
/// both of them or neither. A directory that cannot be read at all becomes a
/// line in the report rather than an early return: the other one may still be
/// there, and it may well be the half that needed elevation — the one the user
/// is least able to redo by hand.
fn restore_all(directories: &[String]) -> backup::RestoreReport {
    let mut report = backup::RestoreReport::default();
    for directory in directories {
        match backup::restore(std::path::Path::new(directory)) {
            Ok(one) => report.merge(one),
            Err(error) => report.failures.push(format!("{directory}: {error:#}")),
        }
    }
    report
}

/// What a restore has to say for itself, in the language on screen.
///
/// The counts first, then one line per key that did not come back. A restore
/// used to stop at its first gap and raise that one file name, so "38 of 43"
/// and "43 of 43" looked exactly alike from the outside.
fn restore_message(
    report: &backup::RestoreReport,
    tr: &'static Strings,
    language: Language,
) -> String {
    let mut message = match report.failed() {
        0 => tr.fmt_restored.replace("{}", &report.restored.to_string()),
        failed => tr
            .fmt_restored_partly
            .replacen("{}", &report.restored.to_string(), 1)
            .replacen("{}", &failed.to_string(), 1),
    };

    if report.removed > 0 {
        message.push('\n');
        message.push_str(
            &tr.fmt_restore_removed
                .replace("{}", &report.removed.to_string()),
        );
    }

    for failure in &report.failures {
        message.push('\n');
        message.push_str(&crate::bilingual::pick(failure, language));
    }

    message
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
                Tab::Backups | Tab::Services => None,
                Tab::Categories | Tab::FileTypes | Tab::Programs => self.handle_keys(&ctx),
            },
        };

        self.report_theme_once(ui);
        self.poll_action(&ctx);
        self.poll_services();
        self.poll_update(&ctx);

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
                self.backup_detail_panel(ui);
                egui::CentralPanel::default().show(ui, |ui| self.backup_list(ui));
            }
            Tab::Favourites => {
                egui::CentralPanel::default()
                    .show(ui, |ui| self.favourite_list(ui, favourite_action));
            }
            Tab::Services => {
                self.service_list(ui);
                egui::CentralPanel::default().show(ui, |ui| self.service_tools_panel(ui));
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

        // After every panel has had its chance to note what the pointer was
        // over, and on every tab: a file can be dropped anywhere in the window.
        self.take_dropped_files(&ctx);

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
        let mut switch_menu: Option<bool> = None;
        let mut switch_handler: Option<bool> = None;
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (Tab::Categories, self.tr.tab_categories),
                    (Tab::FileTypes, self.tr.tab_filetypes),
                    (Tab::Programs, self.tr.tab_programs),
                    (Tab::Favourites, self.tr.tab_favourites),
                    (Tab::Services, self.tr.tab_services),
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
                        if tab == Tab::Services {
                            self.reload_services();
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
                let on_entries =
                    !matches!(self.tab, Tab::Backups | Tab::Favourites | Tab::Services);
                if ui
                    .add_enabled(
                        on_entries,
                        egui::Button::new(labelled(self.glyphs.new, self.tr.editor_new)),
                    )
                    .on_hover_text(self.tr.tip_editor_new)
                    .on_disabled_hover_text(self.tr.tip_entry_tabs_only)
                    .clicked()
                {
                    // Through the same door as every right-click menu, so a new
                    // entry is set up one way and not five.
                    let category = self.category_for_new();
                    self.open_editor_for(category);
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

                // Windows reads the context menu keys once, when Explorer
                // starts. Until now the restart was only offered right after a
                // change -- but the entry that does not show up is noticed
                // later, and then there was no way to it except the task
                // manager. Enabled everywhere, because it has nothing to do
                // with the selection or the tab.
                if ui
                    .button(labelled(self.glyphs.explorer, self.tr.btn_restart_explorer))
                    .on_hover_text(self.tr.tip_restart_explorer)
                    .clicked()
                {
                    match elevation::restart_explorer() {
                        Ok(()) => {
                            self.dialog = Some(Dialog::Note(self.tr.msg_explorer_back.into()))
                        }
                        Err(error) => self.dialog = Some(Dialog::Error(format!("{error:#}"))),
                    }
                }

                // Only where it means something. On Windows 10 this key would
                // be a control that changes nothing, which is worse than no
                // control at all. Read once at startup, not per frame.
                if self.win11 {
                    ui.separator();
                    let classic = self.classic_menu;
                    ui.label(self.tr.menu_style)
                        .on_hover_text(self.tr.tip_menu_style);
                    // No extra mode badge here: it repeated what the lit
                    // switch button already says, and two "Win11" next to
                    // each other confused more than they told (2026-08-20).
                    // The buttons name the menu — "New menu"/"Classic" — and
                    // which row belongs to which menu is the row's badge.
                    for (wanted, label) in [
                        (false, self.tr.menu_style_new),
                        (true, self.tr.menu_style_classic),
                    ] {
                        if ui
                            .add(egui::Button::selectable(classic == wanted, label))
                            .on_hover_text(match wanted {
                                true => self.tr.tip_menu_classic,
                                false => self.tr.tip_menu_new,
                            })
                            .clicked()
                            && classic != wanted
                        {
                            switch_menu = Some(wanted);
                        }
                    }
                    // Own entries in the upper menu. A checkbox rather than a
                    // third selectable button: beside the two-button menu
                    // switch, another lit button read as a third menu mode
                    // (2026-08-20). The checkbox edits a copy — the real
                    // state changes only after the registration succeeded,
                    // so a declined UAC prompt snaps the tick back.
                    let mut wanted = self.handler_installed;
                    if ui
                        .checkbox(&mut wanted, self.tr.btn_handler)
                        .on_hover_text(match self.handler_installed {
                            true => self.tr.tip_handler_remove,
                            false => self.tr.tip_handler_install,
                        })
                        .changed()
                    {
                        switch_handler = Some(wanted);
                    }
                }

                let search = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text(self.tr.search_hint)
                            .desired_width(260.0),
                    )
                    .on_hover_text(self.tr.tip_search);
                // Rebuilding on `changed()` instead of every frame is what
                // keeps typing responsive at a few thousand rows.
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
                    let logo_button = ui.add(
                        egui::Button::image(
                            egui::Image::from_texture(&logo)
                                .fit_to_exact_size(egui::vec2(width, height))
                                .tint(tint),
                        )
                        .frame(false),
                    );
                    // A dot in the corner, and nothing else. The update state
                    // lives in the About window; out here it gets one mark on
                    // the button that opens it, because a bar that is otherwise
                    // all icons has no room for a sentence -- and because a
                    // program that opens a window at its user to announce a
                    // version is a program they learn to dismiss unread.
                    let logo_button = match &self.update {
                        UpdateState::Available(found) => {
                            let corner = logo_button.rect.right_top() + egui::vec2(-1.0, 1.0);
                            ui.painter()
                                .circle_filled(corner, 4.0, ui.visuals().warn_fg_color);
                            logo_button.on_hover_text(
                                self.tr
                                    .fmt_tip_update_available
                                    .replace("{}", &found.version),
                            )
                        }
                        _ => logo_button.on_hover_text(self.tr.tip_about),
                    };
                    if logo_button.clicked() {
                        self.dialog = Some(Dialog::About);
                    }
                    self.settings_controls(ui, ctx);
                });
            });
            ui.add_space(4.0);
        });

        // Outside the panel, because it opens a dialog and touches the
        // registry: doing that inside the closure would borrow `self` twice.
        if let Some(classic) = switch_menu {
            match crate::registry::win11::set_classic_menu(classic) {
                Ok(()) => {
                    self.classic_menu = classic;
                    // The handler is loaded when the shell starts, so nothing
                    // changes until it does. Offered rather than done: a
                    // restart closes every Explorer window the user has open.
                    self.dialog = Some(Dialog::Created {
                        path: String::new(),
                        note: None,
                    });
                }
                Err(error) => self.dialog = Some(Dialog::Error(format!("{error:#}"))),
            }
        }

        if let Some(wanted) = switch_handler {
            // Runs on the frame thread and blocks it, like the elevated half
            // of a plan does: the UAC prompt is modal for the user anyway,
            // and a frozen frame behind a system dialog is the lesser evil
            // than an install whose result arrives when nobody looks.
            let outcome = match wanted {
                true => crate::handler::install(),
                false => crate::handler::remove(),
            };
            match outcome {
                Ok(true) => {
                    self.handler_installed = crate::handler::is_installed();
                    self.dialog = Some(Dialog::Note(
                        match wanted {
                            true => self.tr.msg_handler_installed,
                            false => self.tr.msg_handler_removed,
                        }
                        .to_string(),
                    ));
                }
                // The UAC prompt was declined — a decision, not a fault, and
                // not worth a window of its own.
                Ok(false) => {}
                Err(error) => self.dialog = Some(Dialog::Error(format!("{error:#}"))),
            }
        }
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

    /// Writes what the two settings controls cost the toolbar, once.
    ///
    /// The union of the two rectangles, so the gap between them counts as well:
    /// what the rest of the bar gets back is the whole block, not two widths
    /// added up.
    fn report_settings_width(&mut self, theme: egui::Rect, language: egui::Rect) {
        if self.settings_width_reported {
            return;
        }
        self.settings_width_reported = true;

        crate::errln!(
            "toolbar_settings_width_pt={:.1} theme={:.1} language={:.1}",
            theme.union(language).width(),
            theme.width(),
            language.width()
        );
        crate::console::flush();
    }

    /// Theme and language, as two buttons that show what they are set to.
    ///
    /// Both were drop-downs until 2026-08-16, and between them they held 208
    /// points of the one row that has to fit everything at once — a row that
    /// already clips the search box on a narrow window. A drop-down is the
    /// wrong shape for either of them: the language has exactly two states, and
    /// the theme has three that make a ring. So each became a button that wears
    /// its current state as a picture and steps to the next one when pressed.
    ///
    /// What a picture cannot do is say what it means, which is why both
    /// tooltips name the state *and* the next click rather than the control.
    fn settings_controls(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let mut changed = false;

        let theme = ui
            .button(theme_glyph(self.settings.theme, &self.glyphs).to_string())
            .on_hover_text(theme_tip(self.settings.theme, self.tr));
        if theme.clicked() {
            self.settings.theme = self.settings.theme.next();
            changed = true;
        }

        let language = flag_button(ui, self.settings.language, theme.rect.height())
            .on_hover_text(self.tr.tip_language);
        if language.clicked() {
            self.settings.language = self.settings.language.other();
            changed = true;
        }

        self.report_settings_width(theme.rect, language.rect);

        if changed {
            // Language switching is a single assignment; it takes effect on
            // the next frame with no restart.
            self.tr = strings_for(self.settings.language);
            crate::bilingual::set_language(self.settings.language);
            ctx.set_theme(self.settings.theme.to_preference());
            // Force the title bar to be re-evaluated on the next frame.
            self.titlebar_dark = None;
            let _ = self.settings.save();
        }
    }

    /// The actions, offered from gentle to harsh.
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
        if matches!(self.tab, Tab::Backups | Tab::Favourites | Tab::Services) {
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
                    // Icons only, like the switches: the name of each one is in
                    // its tooltip, which already had to carry the explanation.
                    if ui
                        .button(glyphs.select_all.to_string())
                        .on_hover_text(format!("{} — {}", tr.btn_select_all, tr.tip_select_all))
                        .clicked()
                    {
                        select_all = true;
                    }
                    let deselect = format!("{} — {}", tr.btn_select_none, tr.tip_select_none);
                    if ui
                        .add_enabled(any, egui::Button::new(glyphs.select_none.to_string()))
                        .on_hover_text(&deselect)
                        .on_disabled_hover_text(&deselect)
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
                    // The one place a word stays: this is the single action a
                    // backup cannot be shrugged off for, and an icon alone is
                    // a thin warning for something irreversible.
                    let delete = egui::Button::new(
                        egui::RichText::new(labelled(glyphs.delete, tr.btn_delete))
                            .color(ui.visuals().error_fg_color),
                    );
                    // The reassurance moved in here from a line of its own: it
                    // was a sentence of permanent text in a bar that has to fit
                    // a small screen, and it belongs to this button anyway.
                    let deleting = format!("{}\n\n{}", tr.tip_delete, tr.msg_backup_first);
                    // A packaged entry has no key to delete; the plan path
                    // refuses it too, this merely says so before the click.
                    let deletable = any && state.packaged == 0;
                    if ui
                        .add_enabled(deletable, delete)
                        .on_hover_text(&deleting)
                        .on_disabled_hover_text(match (any, state.packaged > 0) {
                            (true, true) => tr.tip_not_for_packaged.to_string(),
                            (true, false) => deleting.clone(),
                            _ => tr.tip_needs_selection.to_string(),
                        })
                        .clicked()
                    {
                        wanted = Some(Action::Delete);
                    }

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

    /// The tool box.
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
                        before: None,
                    });
                }
            });
        });
        ui.separator();

        if let Some(error) = &self.favourite_error {
            let error = crate::bilingual::pick(error, self.settings.language);
            ui.colored_label(ui.visuals().error_fg_color, error.as_ref());
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

                    // Only the text on the left makes a row current, not the
                    // whole row. A click area over the entire row is registered
                    // after the buttons inside it and therefore lies on top of
                    // them: egui resolves a press against the last thing drawn
                    // over that point, so the buttons never saw a click at all
                    // and Edit and Remove did nothing. Asking afterwards
                    // whether a button had been hit could not work either —
                    // none of them was ever told.
                    let mut name_area = None;

                    let row = frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let text = ui.vertical(|ui| {
                                ui.set_min_width(340.0);
                                ui.strong(&favourite.name);
                                ui.small(describe(favourite, self.tr));
                            });
                            name_area = Some(text.response.rect);

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
                    // of moving through the list agree on where "here" is —
                    // but only when the click was not meant for a button.
                    if let Some(rect) = name_area
                        && ui
                            .interact(rect, ui.id().with(("fav-row", index)), egui::Sense::click())
                            .clicked()
                    {
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

    // -----------------------------------------------------------------------
    // Services
    // -----------------------------------------------------------------------

    fn reload_services(&mut self) {
        (self.services, self.service_error) = services_from_load(service::load());
    }

    /// What clicking a service's name does: mark it, and go and read what it
    /// offers.
    ///
    /// One function rather than the two lines at each call site, because
    /// `--service` has to do exactly what the click does — a second copy that
    /// set the focus and forgot the fetch would show a selected service with
    /// an empty panel beside it, which is the picture this argument exists to
    /// get rid of.
    fn select_service(&mut self, index: usize, ctx: &egui::Context) {
        self.service_focus = Some(index);
        self.start_service_fetch(index, ctx);
    }

    /// Fetches one service's description on a thread.
    ///
    /// On a thread because it is six requests in the worst case over a network
    /// this program knows nothing about, and the frame path may not wait for
    /// that. Measured against SnapOtter on 2026-08-15: 351 paths,
    /// several megabytes of JSON.
    fn start_service_fetch(&mut self, index: usize, ctx: &egui::Context) {
        let Some(service) = self.services.get(index).cloned() else {
            return;
        };
        self.service_tools.clear();
        self.service_open = None;
        self.service_error = None;
        clear_service_inputs(
            &mut self.service_picked,
            &mut self.service_settings,
            &mut self.service_fields,
        );

        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let outcome = service::tools_of(&service).map_err(|error| format!("{error:#}"));
            let _ = tx.send(outcome);
            ctx.request_repaint();
        });
        self.service_rx = Some(rx);
    }

    fn poll_services(&mut self) {
        let Some(rx) = &self.service_rx else { return };
        let Ok(outcome) = rx.try_recv() else { return };
        self.service_rx = None;

        match outcome {
            Ok((address, tools)) => {
                // Once, over the complete description, before anything filters
                // it. Costs one pass over a few hundred paths.
                self.service_grouping = Some(
                    grouping::Grouping::infer(&tools).with_other_label(self.tr.svc_group_other),
                );
                self.service_tools = tools;
                // The address that answered is kept, so the next refresh is one
                // request instead of six guesses.
                if let Some(index) = self.service_focus
                    && let Some(service) = self.services.get_mut(index)
                    && service.spec_url != address
                {
                    service.spec_url = address;
                    let _ = service::save(&self.services);
                }
            }
            Err(error) => self.service_error = Some(error),
        }
    }

    /// Asks GitHub whether there is a newer release.
    ///
    /// On a thread, like every other request in this program: it is a network
    /// call, and the frame path may not wait for one. Called once on start when
    /// the setting allows it, and by the button in the About window whether it
    /// does or not — pressing it *is* the decision the setting otherwise makes.
    fn start_update_check(&mut self, ctx: &egui::Context) {
        if self.update.busy() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let found = crate::update::check().map_err(|error| format!("{error:#}"));
            let _ = tx.send(UpdateMessage::Checked(found));
            ctx.request_repaint();
        });
        self.update = UpdateState::Checking;
        self.update_rx = Some(rx);
    }

    /// Fetches the new version, checks it, and puts it in place of this one.
    ///
    /// Everything that decides whether those bytes are trustworthy happens in
    /// [`crate::update::download`], on this thread, before a single byte is
    /// written anywhere. What comes back here is either a path or a sentence.
    fn start_update_install(&mut self, ctx: &egui::Context) {
        let UpdateState::Available(found) = &self.update else {
            return;
        };
        let found = found.clone();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let installed = crate::update::download(&found)
                .and_then(|bytes| crate::update::install(&bytes))
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(UpdateMessage::Installed(installed));
            ctx.request_repaint();
        });
        self.update = UpdateState::Downloading;
        self.update_rx = Some(rx);
    }

    /// Picks up what the update worker found. Never blocks.
    fn poll_update(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.update_rx else { return };
        let message = match rx.try_recv() {
            Ok(message) => message,
            // A dead channel has to end the wait, or both buttons stay greyed
            // out for the rest of the session — the same lesson as `poll_scan`.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.update_rx = None;
                if self.update.busy() {
                    self.update = UpdateState::Unknown;
                }
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
        };
        self.update_rx = None;

        match message {
            UpdateMessage::Checked(Ok(outcome)) => {
                self.update = match outcome {
                    crate::update::Outcome::Current => UpdateState::Current,
                    crate::update::Outcome::Available(found) => UpdateState::Available(found),
                    crate::update::Outcome::Incomplete(version) => UpdateState::Incomplete(version),
                }
            }
            UpdateMessage::Checked(Err(error)) | UpdateMessage::Installed(Err(error)) => {
                self.fail_update(error);
            }
            UpdateMessage::Installed(Ok(path)) => match crate::update::relaunch(&path) {
                Ok(()) => {
                    self.update = UpdateState::Restarting;
                    // The old file is already renamed and the new one is
                    // already running; staying open would leave two windows.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                // The rare one worth spelling out: the update *is* installed,
                // and only the restart failed. Saying "failed" without that
                // would send someone looking for a problem they no longer have.
                Err(error) => self.fail_update(format!(
                    "{error:#} \u{1e}\u{2014} die neue Fassung liegt bereits an ihrem Platz und startet beim n\u{e4}chsten Mal\u{1f}\u{2014} the new version is already in place and will start next time\u{1d}"
                )),
            },
        }
    }

    /// Records a failed update where the user can find it later.
    ///
    /// Logged as well as shown, because the About window is not where anyone
    /// looks after the fact — and the automatic check on start fails silently
    /// by design, so the log is the only trace it leaves at all.
    fn fail_update(&mut self, error: String) {
        crate::log::write(
            crate::log::Kind::Error,
            &crate::bilingual::pick(&error, self.settings.language),
        );
        self.update = UpdateState::Failed(error);
    }

    /// The update part of the About window.
    ///
    /// Draws into the window's own closure and therefore may not touch `self`
    /// mutably: what a click means comes back through the return value, the
    /// same way the self-entry buttons do it.
    fn update_section(&self, ui: &mut egui::Ui, on_start: &mut bool) -> Option<UpdateAction> {
        let mut action = None;

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.small(self.tr.update_heading);
        ui.add_space(4.0);

        ui.checkbox(on_start, self.tr.update_on_start)
            .on_hover_text(self.tr.tip_update_on_start);

        ui.add_space(4.0);
        match &self.update {
            UpdateState::Unknown => {}
            UpdateState::Checking => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(self.tr.update_checking);
                });
            }
            UpdateState::Current => {
                ui.label(self.tr.update_current);
            }
            UpdateState::Incomplete(version) => {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.label(self.tr.fmt_update_incomplete.replace("{}", version));
            }
            UpdateState::Downloading => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(self.tr.update_downloading);
                });
            }
            UpdateState::Restarting => {
                ui.label(self.tr.update_restarting);
            }
            UpdateState::Failed(error) => {
                let shown = crate::bilingual::pick(error, self.settings.language);
                ui.colored_label(ui.visuals().error_fg_color, shown.as_ref());
            }
            UpdateState::Available(found) => {
                ui.label(
                    egui::RichText::new(self.tr.fmt_update_available.replace("{}", &found.version))
                        .strong(),
                );
                let notes = plain_notes(&found.notes);
                if !notes.is_empty() {
                    ui.add_space(4.0);
                    ui.small(self.tr.update_notes);
                    // In a framed box of its own, and left aligned inside it.
                    // Everything else in this window is centred, which is right
                    // for a name and a version and wrong for five lines of
                    // changelog: centred body text has no left edge for the eye
                    // to return to. The frame is what says the text continues
                    // below the fold -- without it the last line simply stops
                    // mid-sentence and reads as a drawing fault.
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(140.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                    ui.label(notes);
                                });
                            });
                    });
                }
                ui.add_space(6.0);
                if ui
                    .button(self.tr.update_install)
                    .on_hover_text(self.tr.tip_update_install)
                    .clicked()
                {
                    action = Some(UpdateAction::Install);
                }
            }
        }

        ui.add_space(4.0);
        if ui
            .add_enabled(
                !self.update.busy(),
                egui::Button::new(self.tr.update_check_now),
            )
            .clicked()
        {
            action = Some(UpdateAction::Check);
        }

        action
    }

    /// Left panel of the services tab: which service, and the buttons for it.
    fn service_list(&mut self, ui: &mut Ui) {
        let mut fetch = None;
        let mut edit = None;
        let mut remove = None;
        let mut new_service = false;

        egui::Panel::left("services")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.heading(self.tr.tab_services);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(self.tr.svc_new)
                            .on_hover_text(self.tr.tip_svc_new)
                            .clicked()
                        {
                            new_service = true;
                        }
                    });
                });
                ui.separator();

                if self.services.is_empty() {
                    ui.add_space(8.0);
                    ui.label(self.tr.svc_empty);
                    ui.add_space(8.0);
                    // Not `small`: this is the text that explains what the tab
                    // is for, and the one place someone actually reads. It was
                    // already reported once as too small elsewhere.
                    ui.label(self.tr.svc_empty_hint);
                    return;
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (index, service) in self.services.iter().enumerate() {
                            let current = self.service_focus == Some(index);
                            if ui
                                .add(egui::Button::selectable(current, &service.name))
                                .on_hover_text(&service.spec_url)
                                .clicked()
                                && !current
                            {
                                fetch = Some(index);
                            }
                            if current {
                                ui.horizontal(|ui| {
                                    if ui
                                        .small_button(self.tr.svc_refresh)
                                        .on_hover_text(self.tr.tip_svc_refresh)
                                        .clicked()
                                    {
                                        fetch = Some(index);
                                    }
                                    if ui
                                        .small_button(self.tr.fav_edit)
                                        .on_hover_text(self.tr.tip_svc_edit)
                                        .clicked()
                                    {
                                        edit = Some(index);
                                    }
                                    if ui
                                        .small_button(self.tr.fav_remove)
                                        .on_hover_text(self.tr.tip_svc_remove)
                                        .clicked()
                                    {
                                        remove = Some(index);
                                    }
                                });
                            }
                            ui.add_space(2.0);
                        }
                    });
            });

        if new_service {
            self.dialog = Some(Dialog::Service {
                draft: Box::new(blank_service()),
                fresh: true,
            });
        }
        if let Some(index) = edit
            && let Some(service) = self.services.get(index)
        {
            self.dialog = Some(Dialog::Service {
                draft: Box::new(service.clone()),
                fresh: false,
            });
        }
        if let Some(index) = remove {
            self.services.remove(index);
            // The favourites made from it stay: they work on their own, and
            // silently deleting a menu entry the user is still using would be
            // the worst possible reading of "remove this service".
            self.service_focus = None;
            self.service_tools.clear();
            clear_service_inputs(
                &mut self.service_picked,
                &mut self.service_settings,
                &mut self.service_fields,
            );
            if let Err(error) = service::save(&self.services) {
                self.service_error = Some(format!("{error:#}"));
            }
        }
        if let Some(index) = fetch {
            let ctx = ui.ctx().clone();
            self.select_service(index, &ctx);
        }
    }

    /// Centre of the services tab: the tools, grouped the way the service
    /// groups them, with a tick box each.
    fn service_tools_panel(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);

        if let Some(error) = &self.service_error {
            let error = crate::bilingual::pick(error, self.settings.language);
            ui.colored_label(ui.visuals().error_fg_color, error.as_ref());
            ui.separator();
            // Nothing else to say. "This service has no tool that takes a file"
            // underneath a message about the description not being readable
            // reads as a second, contradictory finding.
            return;
        }

        if self.service_focus.is_none() {
            ui.add_space(12.0);
            ui.label(self.tr.svc_pick_service);
            return;
        }
        if self.service_rx.is_some() {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(self.tr.svc_loading);
            });
            return;
        }
        if self.service_tools.is_empty() {
            ui.add_space(12.0);
            ui.label(self.tr.svc_no_tools);
            return;
        }

        let hidden = match self.service_show_async {
            true => 0,
            false => self
                .service_tools
                .iter()
                .filter(|tool| tool.usable == spec::Usable::Asynchronous)
                .count(),
        };
        let listed = self.service_tools.len() - hidden;

        let mut create = false;
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.service_search)
                    .hint_text(self.tr.svc_search_hint)
                    .desired_width(220.0),
            );
            ui.separator();
            ui.label(
                self.tr
                    .fmt_svc_counts
                    .replacen("{}", &listed.to_string(), 1)
                    .replacen("{}", &self.service_picked.len().to_string(), 1),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self.service_picked.is_empty(),
                        egui::Button::new(self.tr.svc_create),
                    )
                    .on_hover_text(self.tr.tip_svc_create)
                    .on_disabled_hover_text(self.tr.tip_svc_create_none)
                    .clicked()
                {
                    create = true;
                }
                if ui
                    .add_enabled(
                        !self.service_picked.is_empty(),
                        egui::Button::new(self.tr.svc_clear),
                    )
                    .clicked()
                {
                    self.service_picked.clear();
                }
            });
        });

        // What was left out, and the way back in. Silently dropping a fifth of
        // a service's endpoints would be the kind of helpfulness nobody can
        // check.
        if hidden > 0 {
            ui.horizontal(|ui| {
                ui.small(
                    self.tr
                        .fmt_svc_async_hidden
                        .replacen("{}", &hidden.to_string(), 1),
                );
                if ui
                    .small_button(self.tr.svc_show_async)
                    .on_hover_text(self.tr.tip_svc_show_async)
                    .clicked()
                {
                    self.service_show_async = true;
                }
            });
        }
        ui.separator();

        let needle = self.service_search.to_lowercase();
        let Some(grouping) = self.service_grouping.clone() else {
            return;
        };
        let groups = group_tools(
            &self.service_tools,
            &needle,
            &grouping,
            self.service_show_async,
        );

        if groups.is_empty() {
            ui.add_space(12.0);
            ui.label(self.tr.svc_nothing_found);
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (position, (tag, indices)) in groups.into_iter().enumerate() {
                    let picked = indices
                        .iter()
                        .filter(|index| self.service_picked.contains(index))
                        .count();
                    let title = match picked {
                        0 => format!("{tag}  ({})", indices.len()),
                        _ => format!("{tag}  ({}/{})", picked, indices.len()),
                    };

                    egui::CollapsingHeader::new(title)
                        .id_salt(("svc-group", &tag))
                        // The first group open, the rest closed, the way the
                        // file type tab already opens Images. Seven closed
                        // headers and nothing else was the whole panel after a
                        // service loaded: a reader saw that 180 tools exist
                        // without seeing a single one of them.
                        .default_open(position == 0)
                        .show(ui, |ui| {
                            // A whole category at once: the reason for grouping
                            // in the first place.
                            ui.horizontal(|ui| {
                                if ui.small_button(self.tr.svc_pick_all).clicked() {
                                    for index in &indices {
                                        if self.service_tools[*index].usable
                                            != spec::Usable::Asynchronous
                                        {
                                            self.service_picked.insert(*index);
                                        }
                                    }
                                }
                                if ui.small_button(self.tr.svc_pick_none).clicked() {
                                    for index in &indices {
                                        self.service_picked.remove(index);
                                    }
                                }
                            });
                            ui.add_space(2.0);

                            for index in indices {
                                self.service_tool_row(ui, index);
                            }
                        });
                }
            });

        if create {
            self.create_picked_tools();
        }
    }

    /// One tool: tick box, name, and whatever it wants filled in.
    fn service_tool_row(&mut self, ui: &mut Ui, index: usize) {
        let tool = self.service_tools[index].clone();
        let usable = tool.usable != spec::Usable::Asynchronous;
        let mut ticked = self.service_picked.contains(&index);

        ui.horizontal(|ui| {
            let box_ = ui.add_enabled(usable, egui::Checkbox::new(&mut ticked, &tool.summary));
            let box_ = match usable {
                true => box_.on_hover_text(&tool.path),
                // Greyed with the reason rather than hidden: "why is this one
                // missing" is a worse question than "why is this one grey".
                false => box_.on_disabled_hover_text(self.tr.tip_svc_async),
            };
            if box_.changed() {
                match ticked {
                    true => self.service_picked.insert(index),
                    false => self.service_picked.remove(&index),
                };
            }

            if !usable {
                ui.small(self.tr.svc_async);
            } else {
                // One tool, one click. Ticking a box and then finding the
                // button that acts on the ticks is two steps for what is
                // usually a decision about a single tool; the boxes stay for
                // the case they were built for, which is a whole category.
                if ui
                    .small_button(self.glyphs.new.to_string())
                    .on_hover_text(self.tr.tip_svc_add_one)
                    .clicked()
                {
                    self.create_one_tool(index);
                }

                if tool.settings != spec::Settings::None {
                    let open = self.service_open == Some(index);
                    if ui
                        .small_button(match open {
                            true => self.tr.svc_settings_hide,
                            false => self.tr.svc_settings_show,
                        })
                        .on_hover_text(self.tr.tip_svc_settings)
                        .clicked()
                    {
                        self.service_open = match open {
                            true => None,
                            false => Some(index),
                        };
                    }
                    if self.service_settings_filled(index, &tool) {
                        ui.small(self.tr.svc_settings_set);
                    }
                }
            }

            // The service's own documentation, at the place this tool sits.
            // Everything the settings need is spelled out there, and rebuilding
            // that in a tooltip would be a worse copy of it.
            if let Some(url) = self.doc_url_for(&tool)
                && ui
                    .small_button(self.glyphs.link.to_string())
                    .on_hover_text(self.tr.tip_svc_docs)
                    .clicked()
            {
                let _ = crate::webtool::shell::open(&url);
            }
        });

        if self.service_open == Some(index) {
            ui.indent(("svc-settings", index), |ui| {
                self.service_settings_form(ui, index, &tool);
            });
        }
    }

    /// Whether anything was typed for this tool.
    fn service_settings_filled(&self, index: usize, tool: &spec::Tool) -> bool {
        match &tool.settings {
            spec::Settings::None => false,
            spec::Settings::Text { .. } => self
                .service_settings
                .get(&index)
                .is_some_and(|value| !value.trim().is_empty()),
            spec::Settings::Fields { fields, .. } => fields.iter().any(|field| {
                self.service_fields
                    .get(&(index, field.name.clone()))
                    .is_some_and(|value| !value.trim().is_empty())
            }),
        }
    }

    /// The form for one tool's settings — typed where the description said
    /// enough to type them, a text box with the service's own prose where it
    /// did not.
    fn service_settings_form(&mut self, ui: &mut Ui, index: usize, tool: &spec::Tool) {
        match &tool.settings {
            spec::Settings::None => {}
            spec::Settings::Text { description, .. } => {
                if let Some(text) = description {
                    // The service's own prose, at reading size: it is the only
                    // place that says what may go in the box below.
                    ui.add(egui::Label::new(shorten(text, 1200)).wrap());
                    ui.add_space(2.0);
                }
                let value = self.service_settings.entry(index).or_default();
                ui.add(
                    egui::TextEdit::multiline(value)
                        .desired_rows(3)
                        .desired_width(760.0)
                        .hint_text(HINT_SETTINGS),
                );
                ui.small(self.tr.svc_settings_json);
            }
            spec::Settings::Fields { fields, .. } => {
                egui::Grid::new(("svc-fields", index))
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        for field in fields {
                            let label = match field.required {
                                true => format!("{} *", field.name),
                                false => field.name.clone(),
                            };
                            let label = ui.label(label);
                            if let Some(text) = &field.description {
                                label.on_hover_text(shorten(text, 400));
                            }

                            let key = (index, field.name.clone());
                            let value = self.service_fields.entry(key).or_default();
                            match &field.kind {
                                spec::FieldKind::Flag => {
                                    let mut on = value == "true";
                                    if ui.checkbox(&mut on, "").changed() {
                                        *value = on.to_string();
                                    }
                                }
                                spec::FieldKind::Choice(options) => {
                                    egui::ComboBox::from_id_salt((
                                        "svc-choice",
                                        index,
                                        &field.name,
                                    ))
                                    .selected_text(value.clone())
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(value, String::new(), "—");
                                        for option in options {
                                            ui.selectable_value(
                                                value,
                                                option.clone(),
                                                option.as_str(),
                                            );
                                        }
                                    });
                                }
                                spec::FieldKind::Number { minimum, maximum } => {
                                    ui.add(
                                        egui::TextEdit::singleline(value)
                                            .desired_width(120.0)
                                            .hint_text(number_hint(*minimum, *maximum)),
                                    );
                                }
                                spec::FieldKind::Text => {
                                    ui.add(egui::TextEdit::singleline(value).desired_width(220.0));
                                }
                            }
                            ui.end_row();
                        }
                    });
            }
        }
    }

    /// Turns every ticked tool into a favourite, in one go.
    fn create_picked_tools(&mut self) {
        let mut picked: Vec<usize> = self.service_picked.iter().copied().collect();
        // The order the description listed them in, so the menu reads the way
        // the service's own documentation does.
        picked.sort_unstable();
        self.create_tools(&picked);
    }

    /// Where this tool is documented, if the service says enough to know.
    fn doc_url_for(&self, tool: &spec::Tool) -> Option<String> {
        let service = self
            .service_focus
            .and_then(|index| self.services.get(index))?;
        Some(docs_url(&service.spec_url, tool))
    }

    /// Makes a favourite of one tool, right away.
    fn create_one_tool(&mut self, index: usize) {
        self.create_tools(&[index]);
    }

    /// The one road both ways of adding a tool travel.
    fn create_tools(&mut self, indices: &[usize]) {
        let Some(service) = self
            .service_focus
            .and_then(|index| self.services.get(index))
        else {
            return;
        };

        let mut made = Vec::new();
        for &index in indices {
            let Some(tool) = self.service_tools.get(index) else {
                continue;
            };
            let settings = match &tool.settings {
                spec::Settings::None => None,
                spec::Settings::Text { .. } => self
                    .service_settings
                    .get(&index)
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                spec::Settings::Fields { fields, .. } => settings_json(fields, &|name| {
                    self.service_fields.get(&(index, name.to_string())).cloned()
                }),
            };
            made.push(service::favourite_for(
                service,
                tool,
                settings,
                SERVICE_SUFFIX,
            ));
        }

        let count = made.len();
        if count == 0 {
            return;
        }
        match favourites::add_many(made) {
            Ok(fresh) => {
                self.reload_favourites();
                for index in indices {
                    self.service_picked.remove(index);
                }
                self.dialog = Some(Dialog::Note(
                    self.tr
                        .fmt_svc_created
                        .replacen("{}", &count.to_string(), 1)
                        .replacen("{}", &(count - fresh).to_string(), 1),
                ));
            }
            Err(error) => self.service_error = Some(format!("{error:#}")),
        }
    }

    /// The form for one service.
    fn service_dialog(&mut self, ui: &mut Ui, mut draft: Box<Service>, fresh: bool) {
        let mut save = false;
        let mut close = false;

        egui::Window::new(match fresh {
            true => self.tr.svc_new,
            false => self.tr.svc_edit,
        })
        .collapsible(false)
        .resizable(true)
        .default_width(620.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ui.ctx(), |ui| {
            let icons = &mut self.icons;
            // A template only fills in what cannot be read off the service
            // itself. It is offered before the fields so it is the first thing
            // tried, not a correction afterwards.
            let mut hint = HINT_SPEC_URL;
            ui.horizontal(|ui| {
                ui.label(self.tr.svc_template)
                    .on_hover_text(self.tr.tip_svc_template);
                for template in service::TEMPLATES {
                    if template.name.is_empty() {
                        continue;
                    }
                    if draft.name.trim() == template.name {
                        hint = template.address_hint;
                    }
                    if ui.small_button(template.name).clicked() {
                        draft.name = template.name.to_string();
                        // Into the field, not only the hint — asked for on
                        // 2026-08-20: `<host>` stays visible for the user
                        // to replace, everything around it is already right.
                        draft.spec_url = template.address_hint.to_string();
                        draft.result_path = template.result_path.to_string();
                        draft.allow_insecure = template.allow_insecure;
                        if !template.icon.is_empty() {
                            draft.icon = Some(template.icon.to_string());
                        }
                    }
                }
            });
            ui.add_space(4.0);

            egui::Grid::new("service-form")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label(self.tr.svc_name);
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.name)
                            .desired_width(420.0)
                            .hint_text(HINT_SERVICE_NAME),
                    );
                    ui.end_row();

                    ui.label(self.tr.svc_address);
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.spec_url)
                            .desired_width(420.0)
                            .hint_text(hint),
                    );
                    ui.end_row();

                    ui.label("");
                    ui.small(self.tr.svc_address_help);
                    ui.end_row();

                    let mut header = draft.auth_header.take().unwrap_or(Header {
                        name: "Authorization".into(),
                        value: String::new(),
                    });
                    ui.label(self.tr.svc_key);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut header.name)
                                .desired_width(140.0)
                                .hint_text("Authorization"),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut header.value)
                                .desired_width(300.0)
                                .hint_text(HINT_KEY),
                        );
                    });
                    ui.end_row();
                    // An empty value means no key at all, which is the normal
                    // case for a public service.
                    draft.auth_header = match header.value.trim().is_empty() {
                        true => None,
                        false => Some(header),
                    };

                    ui.label("");
                    ui.small(self.tr.svc_key_help);
                    ui.end_row();

                    ui.label(self.tr.svc_result);
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.result_path)
                            .desired_width(220.0)
                            .hint_text(HINT_RESULT_PATH),
                    );
                    ui.end_row();

                    ui.label("");
                    ui.small(self.tr.svc_result_help);
                    ui.end_row();

                    // Inherited by every tool taken over from this service —
                    // one face per service, set once (like `result_path`).
                    icon_row(ui, self.tr, icons, 272.0, &mut draft.icon);
                });

            ui.add_space(4.0);
            ui.checkbox(&mut draft.allow_insecure, self.tr.fav_allow_insecure);
            ui.separator();

            ui.horizontal(|ui| {
                let ready = !draft.name.trim().is_empty() && !draft.spec_url.trim().is_empty();
                if ui
                    .add_enabled(ready, egui::Button::new(self.tr.fav_save))
                    .on_disabled_hover_text(self.tr.svc_needs_name)
                    .clicked()
                {
                    save = true;
                }
                if ui.button(self.tr.btn_cancel).clicked() {
                    close = true;
                }
            });
        });

        match (save, close) {
            (true, _) => {
                let mut service = *draft;
                service.name = service.name.trim().to_string();
                service.spec_url = service.spec_url.trim().to_string();
                if service.id.trim().is_empty() {
                    service.id = service::id_for(&service.name);
                }
                // An icon given as a web address becomes a local `.ico` now,
                // not on first use: the registry and the menu read files. A
                // failed fetch stops the save — the same error dialog the
                // entry editor answers with, at the price of the form.
                match service
                    .icon
                    .as_deref()
                    .map(crate::icons::web::localise)
                    .transpose()
                {
                    Err(error) => {
                        self.dialog = Some(Dialog::Error(format!("{error:#}")));
                        return;
                    }
                    Ok(icon) => service.icon = icon.filter(|icon| !icon.is_empty()),
                }

                let index = match self.services.iter().position(|old| old.id == service.id) {
                    Some(index) => {
                        self.services[index] = service;
                        index
                    }
                    None => {
                        self.services.push(service);
                        self.services.len() - 1
                    }
                };
                match service::save(&self.services) {
                    Ok(()) => {
                        self.dialog = None;
                        self.service_focus = Some(index);
                        // Straight into fetching: adding a service and then
                        // having to press a second button to see what it can do
                        // is a step with no decision in it.
                        let ctx = ui.ctx().clone();
                        self.start_service_fetch(index, &ctx);
                    }
                    Err(error) => self.service_error = Some(format!("{error:#}")),
                }
            }
            (_, true) => self.dialog = None,
            _ => self.dialog = Some(Dialog::Service { draft, fresh }),
        }
    }

    /// The dialog that edits one favourite.
    fn favourite_dialog(
        &mut self,
        ui: &mut Ui,
        mut draft: Box<Favourite>,
        before: Option<Box<Favourite>>,
    ) {
        let mut save = false;
        let mut close = false;
        let fresh = before.is_none();

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

                    icon_row(ui, self.tr, icons, field_width, &mut draft.icon);

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
            // A web address in the icon field is fetched into a local `.ico`
            // first; its failure travels the same channel every other
            // favourite failure does and shows up above the list.
            let outcome = draft
                .icon
                .as_deref()
                .map(crate::icons::web::localise)
                .transpose()
                .and_then(|icon| {
                    draft.icon = icon.filter(|icon| !icon.is_empty());
                    match &before {
                        None => favourites::add(*draft.clone()).map(|_| ()),
                        Some(before) => favourites::update(*draft.clone(), before),
                    }
                });
            self.after_favourite_change(outcome);
        } else if !close {
            self.dialog = Some(Dialog::Favourite { draft, before });
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
                Ok(made) => {
                    elevation::notify_shell();
                    self.start_scan(ctx);
                    // `Created`, not `Error`. Announcing a finished entry in a
                    // red window titled "Error", under a button that says
                    // Cancel, tells the user the opposite of what happened --
                    // and this one also carries the question that always
                    // follows, which is whether to restart Explorer.
                    self.dialog = Some(Dialog::Created {
                        path: made.target.full_path(),
                        note: made.note,
                    });
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
                let mut restore: Option<Vec<String>> = None;
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

                        if !report.backup_directories.is_empty() {
                            ui.add_space(4.0);
                            // Every one of them: a split action backs up twice,
                            // and the second directory is the one the
                            // machine-wide changes hang on. Naming only the
                            // first left those with no way back.
                            for directory in &report.backup_directories {
                                ui.label(self.tr.fmt_backup_created.replace("{}", directory));
                            }
                            // Offered right here, because a partial failure is
                            // exactly when someone wants to go back and is
                            // least inclined to go hunting for the path.
                            if ui
                                .button(self.tr.btn_restore)
                                .on_hover_text(self.tr.tip_restore)
                                .clicked()
                            {
                                restore = Some(report.backup_directories.clone());
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
                                            let error = crate::bilingual::pick(
                                                error,
                                                self.settings.language,
                                            );
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

                if let Some(directories) = restore {
                    let restored = restore_all(&directories);
                    match restored.failed() {
                        0 => {
                            self.start_scan(ctx);
                            close = true;
                        }
                        // Said out loud rather than swallowed: what did not
                        // come back is exactly what the user now has to sort
                        // out by hand, and the counts say how much of it there
                        // is.
                        _ => {
                            self.dialog = Some(Dialog::Error(restore_message(
                                &restored,
                                self.tr,
                                self.settings.language,
                            )));
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

            Dialog::Created { path, note } => {
                let mut close = false;
                let mut restart = false;
                egui::Window::new(self.tr.title_note)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        // An empty path means the message is not about an entry
                        // at all -- the menu switch uses this dialog for its own
                        // announcement, because what follows is the same
                        // question either way.
                        ui.add(
                            egui::Label::new(match path.is_empty() {
                                true => self.tr.msg_menu_switched.to_string(),
                                false => self.tr.fmt_entry_created.replace("{}", &path),
                            })
                            .wrap(),
                        );
                        // The entry is there either way -- this says what went
                        // wrong beside it, in the warning colour rather than
                        // the error one, because nothing about the entry
                        // failed.
                        if let Some(note) = &note {
                            ui.add_space(6.0);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(
                                        crate::bilingual::pick(note, self.settings.language)
                                            .into_owned(),
                                    )
                                    .color(ui.visuals().warn_fg_color),
                                )
                                .wrap(),
                            );
                        }
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
                    self.dialog = Some(Dialog::Created { path, note });
                }
                keep = false;
            }

            Dialog::About => {
                let mut close = false;
                let mut open_url: Option<&str> = None;
                let mut open_log = false;
                let mut toggle_self: Option<(Category, bool)> = None;
                let mut update_action: Option<UpdateAction> = None;
                // Copied out and compared afterwards, because the window's
                // closure holds `self` immutably and a checkbox wants a `&mut
                // bool`. The same reason `toggle_self` above exists.
                let mut check_on_start = self.settings.check_for_updates;
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

                            // Put the program itself in the menu it manages.
                            // The one entry a user cannot make with the editor
                            // without first knowing where their own .exe
                            // lives, and the one that makes the program
                            // reachable from where the question comes up.
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(4.0);
                            ui.small(self.tr.self_entry_intro);
                            ui.add_space(4.0);
                            for (category, present) in create::self_entry_present() {
                                let label = format!(
                                    "{}  \u{b7}  {}",
                                    category_label(&category, self.tr),
                                    match present {
                                        true => self.tr.self_entry_there,
                                        false => self.tr.self_entry_absent,
                                    }
                                );
                                ui.horizontal(|ui| {
                                    ui.label(label);
                                    let button = match present {
                                        true => self.tr.self_entry_remove,
                                        false => self.tr.self_entry_add,
                                    };
                                    if ui.small_button(button).clicked() {
                                        toggle_self = Some((category.clone(), present));
                                    }
                                });
                            }

                            update_action = self.update_section(ui, &mut check_on_start);

                            // The log, from the one window every user finds.
                            // A path in a bug report is worth more than a
                            // remembered wording, and nobody types
                            // %LOCALAPPDATA% from a dialog they cannot copy.
                            if let Some(log) = crate::log::path() {
                                ui.add_space(6.0);
                                let exists = log.exists();
                                let link = ui
                                    .add_enabled(exists, egui::Link::new(self.tr.about_log))
                                    .on_hover_text(log.display().to_string())
                                    .on_disabled_hover_text(self.tr.tip_log_empty);
                                if link.clicked() {
                                    open_log = true;
                                }
                            }

                            ui.add_space(10.0);
                            if ui.button(self.tr.btn_close).clicked() {
                                close = true;
                            }
                            ui.add_space(2.0);
                        });
                    });
                // Saved the moment it is ticked, not on closing the window:
                // there is no OK button here, and a setting that quietly
                // forgets itself is worse than no setting.
                if check_on_start != self.settings.check_for_updates {
                    self.settings.check_for_updates = check_on_start;
                    if let Err(error) = self.settings.save() {
                        self.dialog = Some(Dialog::Error(format!("{error:#}")));
                        close = true;
                    }
                }
                match update_action {
                    Some(UpdateAction::Check) => self.start_update_check(ui.ctx()),
                    Some(UpdateAction::Install) => self.start_update_install(ui.ctx()),
                    None => {}
                }

                if let Some(url) = open_url
                    && let Err(error) = crate::webtool::shell::open(url)
                {
                    // Rare, but silence here would look exactly like the bug
                    // this replaced: a link that does nothing when clicked.
                    self.dialog = Some(Dialog::Error(format!("{error:#}")));
                    close = true;
                }
                // Selected in Explorer rather than opened: `.log` has no handler
                // on a fresh Windows, so opening it would raise the "how do you
                // want to open this file" dialog. A window with the file
                // highlighted always works, and the folder next to it is where
                // the backups are.
                if open_log
                    && let Some(file) = crate::log::path()
                    && let Err(error) = crate::elevation::show_in_explorer(&file)
                {
                    self.dialog = Some(Dialog::Error(format!("{error:#}")));
                    close = true;
                }

                if let Some((category, present)) = toggle_self {
                    // Always HKCU, like every entry this program writes, so no
                    // elevation and no effect on anyone else's account.
                    let outcome = match present {
                        true => create::self_entry(category, "x")
                            .and_then(|entry| entry.target())
                            .and_then(|target| create::remove_self(&target))
                            .map(|()| None),
                        false => create::self_entry(category, self.tr.self_entry_name)
                            .and_then(|entry| create::create(&entry))
                            .map(|made| made.note),
                    };
                    match outcome {
                        Err(error) => {
                            self.dialog = Some(Dialog::Error(format!("{error:#}")));
                            close = true;
                        }
                        Ok(note) => {
                            elevation::notify_shell();
                            self.start_scan(ctx);
                            // The entry is in the menu; only the record of it
                            // failed. A note, therefore, and not the red box
                            // that used to stand here for a working entry.
                            if let Some(note) = note {
                                self.dialog = Some(Dialog::Note(
                                    crate::bilingual::pick(&note, self.settings.language)
                                        .into_owned(),
                                ));
                                close = true;
                            }
                        }
                    }
                }
                if !close {
                    self.dialog = Some(Dialog::About);
                }
                keep = false;
            }

            Dialog::Error(message) => {
                // One language, decided here rather than at each of the
                // eighteen places that raise an error -- and decided every
                // frame, so switching the language redraws a dialog that is
                // already open. The stored `message` keeps both.
                let shown = crate::bilingual::pick(&message, self.settings.language).into_owned();
                // Logged here rather than at each of the seventeen places that
                // raise one: this is the one point every error passes through,
                // and it is where "the user was actually shown this" becomes
                // true. Guarded against the message that a dialog re-sets every
                // frame -- without the guard the log would fill at sixty lines
                // a second while the window stands open.
                if self.logged_error.as_deref() != Some(message.as_str()) {
                    crate::log::write(crate::log::Kind::Error, &shown);
                    self.logged_error = Some(message.clone());
                }

                let mut close = false;
                let mut copy = false;
                egui::Window::new(self.tr.title_error)
                    .collapsible(false)
                    .resizable(false)
                    .max_width(560.0)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        // Wrapped, because an error carrying a registry path or
                        // a service's answer is longer than a window is wide,
                        // and a message cut off at the edge cannot be acted on.
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&shown).color(ui.visuals().error_fg_color),
                            )
                            .wrap(),
                        );

                        // What the message means, in the cases where the
                        // wording alone leaves the user guessing. The text
                        // above says what went wrong; this says what to do.
                        if let Some(advice) = advice_for(&shown, self.tr) {
                            ui.add_space(6.0);
                            ui.add(egui::Label::new(advice).wrap());
                        }

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            // "Cancel" was wrong here: there is nothing left to
                            // call off, the thing already failed.
                            if ui.button(self.tr.btn_close).clicked() {
                                close = true;
                            }
                            // So the message can go into a bug report without
                            // being typed off the screen.
                            if ui
                                .button(labelled(self.glyphs.copy, self.tr.btn_copy_error))
                                .on_hover_text(self.tr.tip_copy_error)
                                .clicked()
                            {
                                copy = true;
                            }
                        });
                    });
                if copy {
                    ui.ctx().copy_text(shown.clone());
                }
                if !close {
                    self.dialog = Some(Dialog::Error(message));
                }
                keep = false;
            }

            Dialog::Favourite { draft, before } => {
                self.favourite_dialog(ui, draft, before);
                keep = false;
            }

            Dialog::Service { draft, fresh } => {
                self.service_dialog(ui, draft, fresh);
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
                                        let field = ui.add(
                                            egui::TextEdit::singleline(&mut entry.command)
                                                .desired_width(field_width - 26.0)
                                                .hint_text(HINT_COMMAND),
                                        );
                                        // A program dragged onto the field is
                                        // the shortest way to fill it, and the
                                        // one people try before the button.
                                        // Quoted and given the placeholder the
                                        // category needs, exactly as a drop on
                                        // the category itself would be.
                                        if let Some(path) = dropped_on(ui, field.rect) {
                                            entry.command = format!(
                                                r#""{}" "{}""#,
                                                path.display(),
                                                match create::is_background(&entry.category) {
                                                    true => "%V",
                                                    false => "%1",
                                                }
                                            );
                                        }
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
                                    let field = ui.add(
                                        egui::TextEdit::singleline(&mut icon)
                                            .desired_width(field_width - 52.0)
                                            .hint_text(HINT_ICON),
                                    );
                                    let mut icon = icon.trim().to_string();

                                    // Dropped here, a file is an icon source
                                    // rather than a program: `,0` for the same
                                    // reason the picker adds it — a reference is
                                    // split at its last comma, so a path with
                                    // one in it would lose its tail.
                                    if let Some(path) = dropped_on(ui, field.rect) {
                                        icon = format!("{},0", path.display());
                                    }

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
                    } else if let Err(error) = entry
                        .icon
                        .as_deref()
                        .map(crate::icons::web::localise)
                        .transpose()
                        .map(|icon| entry.icon = icon.filter(|icon| !icon.is_empty()))
                    {
                        // A web address in the icon field could not become a
                        // local `.ico`; nothing has touched the registry yet.
                        self.dialog = Some(Dialog::Error(format!("{error:#}")));
                    } else {
                        match create::create(&entry) {
                            Ok(made) => {
                                // Without this the entry exists but the running
                                // Explorer keeps showing yesterday's menu.
                                elevation::notify_shell();
                                self.start_scan(ctx);
                                // The notification is enough for a static verb
                                // and not for a COM handler, and nobody can
                                // tell which case they are in from the outside.
                                // So the question gets asked rather than the
                                // answer assumed.
                                self.dialog = Some(Dialog::Created {
                                    path: made.target.full_path(),
                                    note: made.note,
                                });
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
                    if !matches!(self.tab, Tab::Backups | Tab::Favourites | Tab::Services) {
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
        let dragging = files_in_the_air(ui.ctx());
        let glyphs = self.glyphs;
        let mut new_here: Option<Category> = None;
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
                            // are both at hand, and only while something is
                            // actually in the air — otherwise the last row the
                            // mouse happened to pass would be remembered as a
                            // drop target for the rest of the session.
                            // `rect_contains_pointer` rather than `hovered`:
                            // during a drag from outside the window egui hands
                            // out no hover at all.
                            if dragging && ui.rect_contains_pointer(response.rect) {
                                self.drop_target = Some(category.clone());
                            }

                            // The right button offers what this row is for:
                            // making an entry that lands here. The category is
                            // the row's own, whether or not it is the selected
                            // one — a right-click names its target.
                            response.context_menu(|ui| {
                                new_here = new_entry_menu(ui, &glyphs, self.tr, category.clone())
                                    .or(new_here.take());
                            });

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

        if let Some(category) = new_here {
            self.open_editor_for(category);
        }
    }

    /// Opens the editor on an empty entry in this category.
    ///
    /// The one path every "new entry" reaches: the button in the tab row, and
    /// the right-click menus in all three trees and under the table.
    fn open_editor_for(&mut self, category: Category) {
        let category = creatable_category(&category).unwrap_or_else(|| self.category_for_new());
        self.open_editor(NewEntry {
            category,
            key_name: String::new(),
            display_name: String::new(),
            command: String::new(),
            icon: None,
            position: None,
            extended: false,
            children: Vec::new(),
        });
    }

    /// Puts a form on screen, however it was filled in.
    ///
    /// Empty from a right-click, filled in from a dropped file, filled in with
    /// an example from `--new`: three ways to arrive, one dialog. The record
    /// of earlier entries is read here and once — it is a file, and no file
    /// belongs in the frame path.
    ///
    /// Nothing is written. `create::write` is reached from the button inside
    /// the form and from nowhere else.
    fn open_editor(&mut self, entry: NewEntry) {
        self.dialog = Some(Dialog::Editor {
            entry: Box::new(entry),
            recorded: create::recorded().unwrap_or_default(),
            existing: None,
        });
    }

    /// Turns a file dropped on the window into a filled-in editor form.
    ///
    /// Called once per frame from `ui`, not from the category tree: a file can
    /// be dropped on any tab and on any part of the window, and the tree is
    /// only drawn on one of them. That was the second half of why this did
    /// nothing at first.
    ///
    /// The category comes from whatever the pointer was over while the file was
    /// in the air, so dragging a program onto "Desktop-Hintergrund" produces an
    /// entry for the desktop background — with `%V`, because `%1` is empty
    /// there. Dropped anywhere else, the category is the one already selected,
    /// which is what the "new entry" button uses too.
    ///
    /// Nothing is written: the form opens and waits, exactly as if it had been
    /// filled in by hand.
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
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

        // A drop into a dialog is not a drop onto a category; the editor takes
        // care of its own fields.
        if self.dialog.is_some() {
            return;
        }

        let category = self
            .drop_target
            .take()
            .and_then(|category| creatable_category(&category))
            .unwrap_or_else(|| self.category_for_new());

        self.open_editor(create::from_dropped_file(&path, category));
    }

    /// File types, grouped, with the number of entries each one adds.
    fn file_type_tree(&mut self, ui: &mut Ui) {
        let glyphs = self.glyphs;
        let mut new_here: Option<Category> = None;
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
                // promised from the start: the curated list is 98 types and
                // every machine has far more registered than that -- 1674 on
                // the one this was written on -- while `custom_extensions` was
                // saved to disk from milestone 5 on and nothing ever read it.
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
                                        let row = ui.selectable_label(selected, label);
                                        if row.clicked() {
                                            clicked = Some(info.ext().to_string());
                                        }
                                        // An entry for exactly this extension,
                                        // without having to select it first.
                                        let ext = info.ext().to_string();
                                        row.context_menu(|ui| {
                                            new_here = new_entry_menu(
                                                ui,
                                                &glyphs,
                                                tr,
                                                Category::ExtAssoc(ext.clone()),
                                            )
                                            .or(new_here.take());
                                        });
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
                if let Some(category) = new_here.take() {
                    self.open_editor_for(category);
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
        let glyphs = self.glyphs;
        let default_category = self.category_for_new();
        let mut new_here: Option<Category> = None;
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
                            before: None,
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
                                        // A program is not a category, so "new
                                        // entry here" means the category the
                                        // button in the tab row would use.
                                        // Offered anyway: this list is where
                                        // somebody notices a program is worth
                                        // an entry of its own.
                                        response.context_menu(|ui| {
                                            new_here = new_entry_menu(
                                                ui,
                                                &glyphs,
                                                tr,
                                                default_category.clone(),
                                            )
                                            .or(new_here.take());
                                        });
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

        if let Some(category) = new_here {
            self.open_editor_for(category);
        }
    }

    fn entry_table(&mut self, ui: &mut Ui, scroll_to: Option<usize>) {
        // First, so the rows drawn afterwards sit on top of it and only the
        // space they leave over answers a right-click.
        self.empty_space_menu(ui);

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
                // scrolling list and a slideshow.
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
                            EntryKind::PackagedVerb { .. } => tr.kind_packaged,
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

    /// The right-click offer for the empty space below the last row.
    ///
    /// Claimed **before** the table is built, and that is the whole trick: egui
    /// resolves a click against the last thing drawn over that point, so a
    /// background claimed afterwards would swallow every row. Claimed first, the
    /// rows sit on top of it and only the space they leave over is left to it.
    ///
    /// `ui.interact` and not `ui.response()`: the latter carries `Sense::hover`,
    /// which never sees a right-click at all — which is why the first attempt at
    /// this did nothing.
    fn empty_space_menu(&mut self, ui: &mut Ui) {
        let background = ui.interact(
            ui.max_rect(),
            ui.id().with("table-background"),
            Sense::click(),
        );

        let mut new_here = None;
        let glyphs = self.glyphs;
        let category = self.category_for_new();
        background.context_menu(|ui| {
            new_here = new_entry_menu(ui, &glyphs, self.tr, category.clone());
        });

        if let Some(category) = new_here {
            self.open_editor_for(category);
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
                            EntryKind::PackagedVerb {
                                clsid,
                                package,
                                package_name,
                                dll,
                                blocked_machine,
                            } => {
                                field(ui, self.tr.detail_package, package_name);
                                field(ui, self.tr.detail_package_full, package);
                                field(ui, self.tr.detail_clsid, clsid);
                                if let Some(dll) = dll {
                                    field(ui, self.tr.detail_server, dll);
                                }
                                if *blocked_machine {
                                    ui.colored_label(
                                        ui.visuals().warn_fg_color,
                                        self.tr.badge_blocked,
                                    );
                                }
                                ui.add_space(6.0);
                                ui.label(self.tr.msg_packaged_note);
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
            let error = crate::bilingual::pick(error, self.settings.language);
            ui.colored_label(ui.visuals().error_fg_color, error.as_ref());
            return;
        }

        if self.backups.is_empty() {
            ui.label(self.tr.msg_backup_first);
            return;
        }

        // One line each, and what is in it goes to the detail pane. As a
        // collapsing header this list answered "what is in this backup" by
        // growing to forty lines and pushing every other backup off screen.
        let mut chosen = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (path, manifest) in &self.backups {
                    let selected = self.selected_backup.as_ref() == Some(path);
                    let label = format!(
                        "{}  —  {}  ({})",
                        manifest.created_at.format("%Y-%m-%d %H:%M:%S"),
                        manifest.action,
                        manifest.entries.len()
                    );
                    // The gaps are worth seeing without opening anything: an
                    // incomplete backup is the one thing about this list that
                    // could matter later.
                    let text = match manifest.missing.is_empty() {
                        true => egui::RichText::new(label),
                        false => egui::RichText::new(label).color(ui.visuals().warn_fg_color),
                    };
                    if ui.selectable_label(selected, text).clicked() {
                        chosen = Some(path.clone());
                    }
                }
            });

        if let Some(path) = chosen {
            self.selected_backup = Some(path);
        }
    }

    /// What one backup holds, beside the list that names it.
    fn backup_detail_panel(&mut self, ui: &mut Ui) {
        let mut restore: Option<std::path::PathBuf> = None;
        let mut show = None;

        egui::Panel::right("details")
            .resizable(true)
            .default_size(340.0)
            .size_range(240.0..=600.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.heading(self.tr.detail_title);
                ui.separator();

                let found = self
                    .selected_backup
                    .as_ref()
                    .and_then(|wanted| self.backups.iter().find(|(path, _)| path == wanted));
                let Some((path, manifest)) = found else {
                    ui.label(self.tr.detail_nothing_selected);
                    return;
                };

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        field(
                            ui,
                            self.tr.backup_created,
                            &manifest.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                        );
                        // The action is written by the code that made the
                        // backup and is deliberately not translated: it is also
                        // the directory name, and a name that changed with the
                        // language setting would not find its own folder again.
                        field(ui, self.tr.backup_action, &manifest.action);
                        field(ui, self.tr.backup_directory, &path.display().to_string());

                        ui.horizontal(|ui| {
                            if ui
                                .button(labelled(self.glyphs.restore, self.tr.btn_restore))
                                .on_hover_text(self.tr.tip_restore)
                                .clicked()
                            {
                                restore = Some(path.clone());
                            }
                            if folder_button(
                                ui,
                                &mut self.icons,
                                &self
                                    .tr
                                    .fmt_tip_show_in_explorer
                                    .replace("{}", &path.display().to_string()),
                            ) {
                                show = Some(path.clone());
                            }
                        });

                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "{}  ({})",
                                self.tr.backup_keys,
                                manifest.entries.len()
                            ))
                            .strong(),
                        );
                        for entry in &manifest.entries {
                            ui.add(
                                egui::Label::new(egui::RichText::new(&entry.registry_path).small())
                                    .selectable(true)
                                    .wrap(),
                            );
                        }

                        // Recorded rather than dropped: on restore this is the
                        // difference between "nothing to bring back" and "the
                        // export silently failed".
                        if !manifest.missing.is_empty() {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "{}  ({})",
                                    self.tr.backup_missing,
                                    manifest.missing.len()
                                ))
                                .strong()
                                .color(ui.visuals().warn_fg_color),
                            );
                            for missing in &manifest.missing {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(missing).small().weak())
                                        .wrap(),
                                );
                            }
                        }

                        // What reg.exe said about the gaps, so an incomplete
                        // backup can be judged instead of only noticed.
                        if !manifest.notes.is_empty() {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(self.tr.backup_notes).strong());
                            for note in &manifest.notes {
                                // Cut here as well, for manifests written
                                // before the markers were stripped on the way
                                // in. An old backup is exactly the thing that
                                // still has to read correctly.
                                let note = crate::bilingual::pick(note, self.settings.language);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(note.as_ref()).small().weak(),
                                    )
                                    .wrap(),
                                );
                            }
                        }
                    });
            });

        if let Some(path) = show {
            let _ = elevation::show_in_explorer(&path);
        }

        // After the panel, not inside it: this replaces the very list the
        // closure above is reading.
        if let Some(path) = restore {
            match backup::restore(&path) {
                Ok(report) => {
                    elevation::notify_shell();
                    let message = restore_message(&report, self.tr, self.settings.language);
                    // A restore that only half worked is not a success with a
                    // smaller number in it. It used to stop at the first gap
                    // and raise that one file name; now every key is attempted
                    // and the ones that failed are named.
                    self.dialog = Some(match report.failed() {
                        0 => Dialog::Note(message),
                        _ => Dialog::Error(message),
                    });
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

/// An empty service to start from.
fn blank_service() -> Service {
    Service {
        id: String::new(),
        name: String::new(),
        spec_url: String::new(),
        auth_header: None,
        // A service worth adding this way is usually one on the local network,
        // and that is exactly where there is no certificate.
        allow_insecure: true,
        result_path: String::new(),
        icon: None,
    }
}

/// Drops everything the settings form remembers by index into
/// `service_tools`, so a value typed for one service's tool never resurfaces
/// as the value for another service's identically-indexed tool.
///
/// `service_tools` always starts over from index 0 -- on a fresh fetch and on
/// removing the focused service alike -- so this belongs wherever that
/// happens, alongside the index it depends on: `service_picked` reads the
/// same way and is cleared here too, rather than once more at each call site.
fn clear_service_inputs(
    picked: &mut rustc_hash::FxHashSet<usize>,
    settings: &mut rustc_hash::FxHashMap<usize, String>,
    fields: &mut rustc_hash::FxHashMap<(usize, String), String>,
) {
    picked.clear();
    settings.clear();
    fields.clear();
}

/// What the constructor keeps from loading `services.json`: the list, and an
/// error to show if that failed.
///
/// Pulled out of `App::new` so the "a damaged file must not look like an
/// empty, healthy one" rule has a test that never touches a real
/// `%LOCALAPPDATA%\ctxmenu\services.json`. `reload_services` -- run when the
/// user switches to the tab -- already gets this right; the constructor used
/// to take a shortcut around it with `.unwrap_or_default()`, which is exactly
/// how a load failure went unnoticed on `--tab services`, where the tab-click
/// handler that calls `reload_services` never runs.
fn services_from_load(loaded: anyhow::Result<Vec<Service>>) -> (Vec<Service>, Option<String>) {
    match loaded {
        Ok(list) => (list, None),
        Err(error) => (Vec::new(), Some(format!("{error:#}"))),
    }
}

/// The tools of a service, grouped the way the service groups them.
///
/// Order matters twice over: the groups appear in the order their first tool
/// does, and the tools inside a group keep the order the description listed
/// them in — which is the order the service's own documentation shows.
fn group_tools(
    tools: &[spec::Tool],
    needle: &str,
    grouping: &grouping::Grouping,
    with_async: bool,
) -> Vec<(String, Vec<usize>)> {
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (index, tool) in tools.iter().enumerate() {
        if !with_async && tool.usable == spec::Usable::Asynchronous {
            continue;
        }
        if !needle.is_empty() && !matches_tool(tool, needle) {
            continue;
        }
        // The grouping is asked, never recomputed: it belongs to the whole
        // description, not to whatever the search box left standing.
        let name = grouping.category_of(tool);
        match groups.iter_mut().find(|(existing, _)| *existing == name) {
            Some((_, list)) => list.push(index),
            None => groups.push((name, vec![index])),
        }
    }
    // Biggest first, ties alphabetical: with a hundred tools in one group and
    // one in another, the order they happened to appear in says nothing.
    groups
        .sort_by(|(a_name, a), (b_name, b)| b.len().cmp(&a.len()).then_with(|| a_name.cmp(b_name)));
    groups
}

/// Whether a tool answers a search. `needle` is already lower case.
fn matches_tool(tool: &spec::Tool, needle: &str) -> bool {
    tool.summary.to_lowercase().contains(needle)
        || tool.path.to_lowercase().contains(needle)
        || tool
            .tag
            .as_deref()
            .is_some_and(|tag| tag.to_lowercase().contains(needle))
}

/// The typed fields of a tool as the JSON object the service expects.
///
/// Only what was filled in: leaving a field empty has to mean "the service
/// decides", not "send an empty string" — several of them refuse the latter.
/// Numbers and flags are written as numbers and flags, because a schema that
/// declares `integer` will not take `"1920"`.
fn settings_json(
    fields: &[spec::Field],
    value_of: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let mut object = serde_json::Map::new();
    for field in fields {
        let Some(raw) = value_of(&field.name) else {
            continue;
        };
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let value = match &field.kind {
            spec::FieldKind::Flag => serde_json::Value::Bool(raw == "true"),
            spec::FieldKind::Number { .. } => match raw.parse::<f64>() {
                // A whole number goes out whole. `json!(1024.0_f64)` writes
                // "1024.0", and a service that declared the field as `integer`
                // is entitled to refuse that -- which is exactly the sort of
                // failure that looks like the program sent nothing at all.
                Ok(number) if number.fract() == 0.0 && number.abs() < 9e15 => {
                    serde_json::json!(number as i64)
                }
                Ok(number) => serde_json::json!(number),
                // A number that does not parse travels as the text it is: the
                // service's own error message says more than a silent drop.
                Err(_) => serde_json::Value::String(raw.to_string()),
            },
            _ => serde_json::Value::String(raw.to_string()),
        };
        object.insert(field.name.clone(), value);
    }
    match object.is_empty() {
        true => None,
        false => Some(serde_json::Value::Object(object).to_string()),
    }
}

/// The range a number field accepts, as a hint inside the empty box.
fn number_hint(minimum: Option<f64>, maximum: Option<f64>) -> String {
    match (minimum, maximum) {
        (Some(low), Some(high)) => format!("{low} \u{2013} {high}"),
        (Some(low), None) => format!("\u{2265} {low}"),
        (None, Some(high)) => format!("\u{2264} {high}"),
        (None, None) => String::new(),
    }
}

/// What to do about an error, where the message alone leaves that open.
///
/// The messages below the surface say what the API refused, in the words of the
/// API: "Zugriff verweigert", "5", "der Dienst antwortete mit 401". True, and
/// useless on its own -- the user is left to work out that a key expired or that
/// this key lives under HKLM. So the technical sentence stays exactly as it is,
/// and a second one says what it means here.
///
/// Matched on the wording rather than on typed errors: these come from four
/// layers through `anyhow`, and threading an error kind through all of them
/// would be a rebuild of every signature for the sake of a hint. The cost of a
/// missed match is one missing sentence.
fn advice_for(message: &str, tr: &'static Strings) -> Option<&'static str> {
    let lower = message.to_lowercase();

    // Windows refuses the write itself: either the key belongs to the machine
    // rather than the user, or something else holds it open.
    if lower.contains("zugriff verweigert")
        || lower.contains("access is denied")
        || lower.contains("os error 5")
    {
        return Some(tr.why_access_denied);
    }
    // The service turned the request away rather than failing to answer it.
    if lower.contains(" 401") || lower.contains(" 403") {
        return Some(tr.why_unauthorised);
    }
    if lower.contains(" 404") {
        return Some(tr.why_not_found);
    }
    // WinHTTP could not reach the host at all.
    if lower.contains("winhttpconnect")
        || lower.contains("winhttpsendrequest")
        || lower.contains("timed out")
        || lower.contains("zeit\u{fc}berschreitung")
    {
        return Some(tr.why_unreachable);
    }
    if lower.contains("schon vergeben") || lower.contains("already taken") {
        return Some(tr.why_id_taken);
    }
    if lower.contains("existiert bereits") || lower.contains("already exists") {
        return Some(tr.why_key_exists);
    }
    None
}

/// Where a service documents one of its tools, for a human to read.
///
/// The stored address is the machine readable document -- `…/api/docs/openapi.json`
/// after a fetch. The page people read is its directory, and the two viewers in
/// wide use (Scalar and Swagger UI) both address a single operation with the same
/// fragment: `#tag/<tag>/<method><path>`. A fragment that does not match leaves
/// the reader on the front page of the documentation, which is still the right
/// place -- so guessing costs nothing and usually saves the search.
fn docs_url(spec_url: &str, tool: &spec::Tool) -> String {
    let base = spec_url.trim().split('#').next().unwrap_or("").trim_end();
    let page = match base.rsplit_once('/') {
        // A document, not a page: its directory is what a human opens.
        Some((directory, file))
            if file.ends_with(".json") || file.ends_with(".yaml") || file.ends_with(".yml") =>
        {
            format!("{directory}/")
        }
        _ => base.to_string(),
    };

    match &tool.tag {
        Some(tag) => format!(
            "{page}#tag/{}/{}{}",
            tag.to_lowercase(),
            tool.method.to_uppercase(),
            tool.path
        ),
        None => page,
    }
}

/// Cuts a service's own prose down to something a panel can hold.
fn shorten(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let cut: String = text.chars().take(limit).collect();
    format!("{cut}\u{2026}")
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

/// The icon row three forms share: text field, drop target, file picker,
/// live preview — the entry editor's shape, drawn from one place so a
/// favourite, a service and an entry all speak the same field. Asked for on
/// 2026-08-20, when a service's tools had no way to a picture at all.
fn icon_row(
    ui: &mut Ui,
    tr: &'static Strings,
    icons: &mut IconCache,
    field_width: f32,
    value: &mut Option<String>,
) {
    ui.label(tr.editor_icon);
    ui.horizontal(|ui| {
        let mut icon = value.clone().unwrap_or_default();
        let field = ui.add(
            egui::TextEdit::singleline(&mut icon)
                .desired_width(field_width - 52.0)
                .hint_text(HINT_ICON),
        );
        let mut icon = icon.trim().to_string();

        if let Some(path) = dropped_on(ui, field.rect) {
            icon = format!("{},0", path.display());
        }

        if folder_button(ui, icons, tr.tip_pick_icon)
            && let Some(path) = crate::filedialog::pick_file(None, &crate::filedialog::ICONS, &icon)
        {
            icon = format!("{},0", path.display());
        }

        if !icon.is_empty() {
            let texture = icons.get(&icon).clone();
            ui.add(egui::Image::new(egui::load::SizedTexture::new(
                texture.id(),
                egui::vec2(16.0, 16.0),
            )));
        }
        *value = (!icon.is_empty()).then_some(icon);
    });
    ui.end_row();
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
                    _ => WebMode::Upload(Box::new(Upload {
                        endpoint: url,
                        method: "POST".into(),
                        body: UploadBody::Multipart {
                            field: "file".into(),
                        },
                        headers: Vec::new(),
                        fields: Vec::new(),
                        poll: None,
                        result: ResultAction::Report,
                    })),
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

    // The form fields that travel beside the file. This is where a tool's
    // settings live, and until now they were invisible here: a favourite made
    // on the services tab carried them, and opening it for editing showed no
    // trace of them at all.
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(tr.fav_fields).on_hover_text(tr.tip_fav_fields);
        if ui.small_button(tr.fav_header_add).clicked() {
            upload.fields.push(Header {
                name: String::new(),
                value: String::new(),
            });
        }
    });

    let mut drop_field = None;
    for (index, field) in upload.fields.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut field.name)
                    .desired_width(160.0)
                    .hint_text("settings"),
            );
            // Multiline: the value is usually JSON, and a single line hides
            // everything past the first forty characters of it.
            ui.add(
                egui::TextEdit::multiline(&mut field.value)
                    .desired_width(320.0)
                    .desired_rows(2)
                    .hint_text(HINT_SETTINGS),
            );
            if ui.small_button("\u{00d7}").clicked() {
                drop_field = Some(index);
            }
        });
    }
    if let Some(index) = drop_field {
        upload.fields.remove(index);
    }

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

    // A worked example, folded away. Every field above is obvious once one of
    // these has been filled in and impossible to guess before that: which
    // header carries a key, what a JSON path looks like, why a self-hosted
    // service on the local network needs the insecure box.
    ui.add_space(6.0);
    egui::CollapsingHeader::new(tr.editor_help)
        .id_salt("fav-upload-help")
        .default_open(false)
        .show(ui, |ui| {
            for (label, value) in [
                (tr.fav_endpoint, UPLOAD_EXAMPLE_ENDPOINT),
                (tr.fav_field, UPLOAD_EXAMPLE_FIELD),
                (tr.fav_headers, UPLOAD_EXAMPLE_HEADER),
                (tr.fav_source_json, UPLOAD_EXAMPLE_PATH),
            ] {
                ui.label(egui::RichText::new(label).weak().small());
                ui.add(
                    egui::Label::new(egui::RichText::new(value).monospace())
                        .selectable(true)
                        .wrap(),
                );
                ui.add_space(4.0);
            }
            ui.add(egui::Label::new(tr.fav_help_upload).wrap());
        });
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
///
/// A ProgID's own `from_ext` is a poor guide to that, though: the scanner
/// reads its shared `shell` key once and hands the resulting entry to every
/// extension that lists it, so `from_ext` only records whichever
/// extension the scan happened to reach it through first — not necessarily
/// the one on screen. The file type tab always knows which extension is
/// selected, and that is the one a click on a shared entry means; it wins
/// over the entry's own `from_ext` whenever it is available.
fn category_for_new_entry(
    focused: Option<&ContextEntry>,
    tab: Tab,
    selected_ext: Option<&str>,
    selected_category: Option<&Category>,
) -> Category {
    if let Some(entry) = focused
        && let Category::ProgId { .. } = &entry.category
        && tab == Tab::FileTypes
        && let Some(ext) = selected_ext
    {
        return Category::ExtAssoc(ext.to_string());
    }

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
            // Searchable by what the detail pane shows: the package's name
            // is what a reader remembers, the CLSID what a bug report has.
            EntryKind::PackagedVerb {
                clsid,
                package,
                package_name,
                ..
            } => {
                package_name.to_lowercase().contains(needle)
                    || package.to_lowercase().contains(needle)
                    || clsid.to_lowercase().contains(needle)
            }
        }
}

fn detail_text(entry: &ContextEntry) -> &str {
    match &entry.kind {
        EntryKind::Verb { command, .. } => command.as_deref().unwrap_or("—"),
        EntryKind::ShellEx { clsid, .. } => clsid,
        // The command column: a packaged verb has no command line, the
        // package is the closest thing to "what runs here".
        EntryKind::PackagedVerb { package_name, .. } => package_name,
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
    // Shift rule and position do not exist for entries of the new Windows 11
    // menu; with one of those in the selection the groups go grey and say
    // why, instead of erroring after the click.
    let not_packaged = needs_rows.or((state.packaged > 0).then_some(tr.tip_not_for_packaged));

    if let Some(index) = switch_group(
        ui,
        tr.group_visibility,
        mixed_marker(tr, state.hidden == Agreement::Mixed),
        &group_tip(tr.group_visibility, tr.tip_group_visibility),
        needs_rows,
        &[
            Segment {
                icon: glyphs.visible,
                name: tr.seg_visible,
                current: state.hidden == Agreement::Same(false),
            },
            Segment {
                icon: glyphs.hidden,
                name: tr.seg_hidden,
                current: state.hidden == Agreement::Same(true),
            },
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
        not_packaged,
        &[
            Segment {
                icon: glyphs.always,
                name: tr.seg_always,
                current: state.extended == Agreement::Same(false),
            },
            Segment {
                icon: glyphs.shift_only,
                name: tr.seg_shift_only,
                current: state.extended == Agreement::Same(true),
            },
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
            Segment {
                icon: glyphs.free,
                name: tr.seg_free,
                current: state.blocked == Agreement::Same(false),
            },
            Segment {
                icon: glyphs.blocked,
                name: tr.seg_blocked,
                current: state.blocked == Agreement::Same(true),
            },
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
        not_packaged,
        &[
            Segment {
                icon: glyphs.no_position,
                name: tr.pos_default,
                current: at(None),
            },
            Segment {
                icon: glyphs.top,
                name: tr.pos_top,
                current: at(Some("Top")),
            },
            Segment {
                icon: glyphs.bottom,
                name: tr.pos_bottom,
                current: at(Some("Bottom")),
            },
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
    segments: &[Segment],
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
            for (index, segment) in segments.iter().enumerate() {
                // The icon alone. The word that used to stand beside it moved
                // into the tooltip: with five groups spelled out, the bar was
                // wider than a small screen and wrapped into three lines, and
                // the group title in front already says what is being chosen.
                let explanation = format!("{} — {tip}", segment.name);
                // `Button::selectable`, not `SelectableLabel`: egui 0.36 folded
                // the second into the first, and the tab row above takes the
                // same shape through `ui.selectable_label`.
                let response = ui
                    .add_enabled(
                        reason.is_none(),
                        egui::Button::selectable(segment.current, segment.icon.to_string()),
                    )
                    .on_hover_text(&explanation)
                    // A greyed-out control that explains nothing is the thing
                    // this whole bar was rebuilt to get rid of.
                    .on_disabled_hover_text(reason.unwrap_or(&explanation));
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
fn group_width(ui: &Ui, title: &str, mixed: Option<&str>, segments: &[Segment]) -> f32 {
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
    for segment in segments {
        width += spacing + padding + measure(&segment.icon.to_string(), &body);
    }
    width + spacing
}

/// One choice inside a switch group: the icon shown, the word behind it.
///
/// The word is not drawn any more — it names the segment in the tooltip, which
/// is where it went when the bar had to fit a small screen.
struct Segment {
    icon: char,
    name: &'static str,
    current: bool,
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
    // same RGB is not equally readable in both themes.
    let warn = ui.visuals().warn_fg_color;
    let weak = ui.visuals().weak_text_color();

    // Provenance first, states after: "Win11" says which menu the row
    // belongs to, everything behind it says what is special about it.
    if matches!(entry.kind, EntryKind::PackagedVerb { .. }) {
        ui.label(egui::RichText::new(tr.badge_win11).strong());
    }
    if entry.read_only {
        ui.colored_label(weak, "🔒");
    }
    if entry.hidden {
        ui.colored_label(weak, tr.badge_hidden);
    }
    if entry.extended {
        ui.colored_label(warn, "⇧");
    }
    if let EntryKind::ShellEx { blocked: true, .. }
    | EntryKind::PackagedVerb {
        blocked_machine: true,
        ..
    } = entry.kind
    {
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

    // A packaged entry explains itself in its own words throughout: its
    // "hidden" is a per-user block, not LegacyDisable, and its Machine scope
    // says where the package is registered — changing nothing about the
    // fact that hiding needs no admin. The classic explanations would state
    // the wrong mechanism, so none of them is reused.
    if let EntryKind::PackagedVerb {
        blocked_machine, ..
    } = &entry.kind
    {
        if entry.hidden {
            out.push((tr.badge_hidden, tr.why_hidden_packaged));
        }
        if *blocked_machine {
            out.push((tr.badge_blocked, tr.why_blocked));
        }
        out.push((tr.badge_win11, tr.why_packaged));
        return out;
    }

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
/// system tool. A failed read leaves the default font rather than panicking.
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
pub fn run(start: Start) -> eframe::Result<()> {
    // One line per run, so the entries under it can be told apart -- and so a
    // log that ends without a matching error says "it just closed", which is
    // itself worth knowing.
    let how = match start.synthetic {
        Some(rows) => format!("window, {rows} synthetic rows"),
        None => "window".to_string(),
    };
    crate::log::note_start(&how);

    // The size a run asked for is meant in pixels and lands here as points,
    // which is right on a screen at 100 % and too small on this machine's
    // 150 % ones. `place_window_once` corrects it in the first frame; this is
    // only about opening near the right size instead of visibly jumping to it.
    let (width, height) = match start.size {
        Some((width, height)) => (width as f32, height as f32),
        None => (1200.0, 800.0),
    };

    // OpenGL first, DirectX after. `glow` is smaller and starts faster, and on
    // a machine with a real graphics driver it is the right answer. But there
    // are machines without one: a Hyper-V guest, an RDP session, a fresh
    // install before Windows Update has fetched a driver. Measured in a Windows
    // 11 guest on 2026-08-16 -- DirectX 12 feature level 12_1 present, OpenGL
    // absent -- where the program refused to start at all with "egui_glow
    // requires opengl 2.0+". A program that does not open is worse than a
    // program that opens through a second renderer.
    //
    // Tried in this order rather than probed: asking whether OpenGL 2.0 exists
    // means creating a context, which is most of the work of starting anyway.
    for renderer in [eframe::Renderer::Glow, eframe::Renderer::Wgpu] {
        let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
        // No waiting for a vertical blank on the software path. WARP has no
        // display to synchronise with -- the "blank" it reports is a timer --
        // and `Fifo` makes every present block on it. Measured symptom: the
        // window drags behind the mouse while its contents scroll smoothly,
        // which is presentation stalling rather than drawing being slow.
        //
        // Only in the wgpu branch: on a real driver `Fifo` is right, it is what
        // keeps the frame rate at the refresh rate instead of spinning a GPU
        // for frames nobody sees.
        wgpu_options.surface.present_mode = eframe::wgpu::PresentMode::AutoNoVsync;
        // One frame in flight rather than two: a queued frame that is already
        // stale by the time it appears is exactly the lag being chased here.
        wgpu_options.surface.desired_maximum_frame_latency = Some(1);

        let options = eframe::NativeOptions {
            renderer,
            wgpu_options,
            viewport: egui::ViewportBuilder::default()
                // German here and corrected in the first frame, because the
                // settings are not read yet — `sync_title` puts both the
                // language and the version right before anyone can read this
                // one.
                .with_title(window_title(&i18n::DE))
                .with_inner_size([width, height])
                .with_min_inner_size([900.0, 600.0]),
            ..Default::default()
        };

        // Cloned per attempt: the loop may come round a second time, and the
        // closure has to own what it hands to `App::new`.
        let start = start.clone();
        let outcome = eframe::run_native(
            "ctxmenu",
            options,
            Box::new(move |cc| Ok(Box::new(App::new(cc, start.clone())))),
        );

        match outcome {
            Ok(()) => return Ok(()),
            Err(error) => {
                // Only the second failure is the user's problem; the first one
                // is a fact about their graphics driver, and the log is where
                // that belongs.
                crate::log::write(crate::log::Kind::Error, &format!("{renderer:?}: {error}"));
                if renderer == eframe::Renderer::Wgpu {
                    return Err(error);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic;

    fn rows(indices: &[usize]) -> rustc_hash::FxHashSet<Row> {
        indices.iter().map(|index| Row::top(*index)).collect()
    }

    fn bench(remaining: usize) -> Bench {
        Bench {
            warmup: 0,
            remaining,
            scroll: 0,
            keys_sent: 0,
            cursor_walked: 0,
            last_focus: None,
        }
    }

    /// Regression: `bench.remaining -= 1` ran unconditionally,
    /// so a frame arriving once the counter already reached zero wrapped it
    /// to `usize::MAX` in release, where overflow checks are off. That is
    /// what made `--bench 0` hang forever instead of closing immediately.
    #[test]
    fn count_down_never_wraps_the_counter_past_zero() {
        let mut b = bench(1);

        assert!(b.count_down(), "one frame left: this one counts");
        assert_eq!(b.remaining, 0);

        assert!(!b.count_down(), "already at zero: nothing left to count");
        assert_eq!(b.remaining, 0, "must stay at zero, not wrap to usize::MAX");
    }

    /// A report as the two halves of a split action hand one back.
    fn half(directory: &str, name: &str) -> Report {
        Report {
            backup_directories: vec![directory.into()],
            results: vec![crate::registry::plan::OperationResult {
                display_name: name.into(),
                registry_path: format!("HKCU\\{name}"),
                action: Action::Hide,
                error: None,
            }],
        }
    }

    #[test]
    fn a_successful_elevated_half_survives_a_failed_direct_half() {
        // The bug: the elevated half's report was dropped whenever the direct
        // half returned `Err` -- and by then its changes were on the machine
        // and its backup was on disk. The user saw an error window naming the
        // other half and nothing else.
        let combined = combine_halves(
            Err("Backup-Export fehlgeschlagen".into()),
            Ok(half("erhoeht", "hklm_eintrag")),
            &i18n::DE,
        )
        .expect("half of it worked, and that half must be reported");

        assert_eq!(
            combined.backup_directories,
            vec!["erhoeht".to_string()],
            "the restore button needs the directory the changes hang on"
        );
        assert_eq!(combined.succeeded(), 1);
        assert_eq!(combined.failed(), 1, "the direct half is a failed row");
        assert_eq!(combined.results[0].display_name, i18n::DE.direct_part);
        assert_eq!(
            combined.results[0].error.as_deref(),
            Some("Backup-Export fehlgeschlagen")
        );
    }

    #[test]
    fn both_backup_directories_reach_the_result_dialog() {
        let combined = combine_halves(
            Ok(half("direkt", "hkcu_eintrag")),
            Ok(half("erhoeht", "hklm_eintrag")),
            &i18n::DE,
        )
        .expect("both halves worked");

        assert_eq!(combined.backup_directories, vec!["direkt", "erhoeht"]);
        assert_eq!(combined.succeeded(), 2);
    }

    #[test]
    fn a_declined_prompt_still_keeps_what_was_already_done() {
        // The direction that was always handled, kept under test now that the
        // four cases live in one place.
        let combined = combine_halves(
            Ok(half("direkt", "hkcu_eintrag")),
            Err("Vom Benutzer abgebrochen".into()),
            &i18n::DE,
        )
        .expect("a declined prompt is not a failure of the whole action");

        assert_eq!(combined.succeeded(), 1);
        assert_eq!(combined.results[1].display_name, i18n::DE.elevated_part);

        // Only when neither half ran is there nothing to show -- and then both
        // reasons are given, not one.
        let nothing = combine_halves(Err("erstens".into()), Err("zweitens".into()), &i18n::DE)
            .expect_err("nothing happened at all");
        assert!(nothing.contains("erstens") && nothing.contains("zweitens"));
    }

    fn tool(summary: &str, tag: Option<&str>) -> spec::Tool {
        spec::Tool {
            path: format!("/api/v1/tools/{}", summary.to_lowercase()),
            base: "/".into(),
            progress: String::new(),
            method: "POST".into(),
            tag: tag.map(str::to_string),
            summary: summary.into(),
            description: None,
            file_field: "file".into(),
            settings: spec::Settings::None,
            usable: spec::Usable::Yes,
        }
    }

    /// The grouping of a whole description, the way the panel builds it.
    fn grouped(tools: &[spec::Tool]) -> grouping::Grouping {
        grouping::Grouping::infer(tools).with_other_label("Sonstige")
    }

    #[test]
    fn tools_are_listed_under_the_group_the_description_puts_them_in() {
        let tools = vec![
            tool("Compress", Some("Image")),
            tool("Merge", Some("PDF")),
            tool("Resize", Some("Image")),
            tool("Convert", None),
        ];

        let groups = group_tools(&tools, "", &grouped(&tools), false);

        // Biggest group first, and inside it the order the description listed
        // them in.
        assert_eq!(groups[0].0, "Image");
        assert_eq!(groups[0].1, vec![0, 2]);
        // Nothing is lost on the way into the groups.
        assert_eq!(groups.iter().map(|(_, list)| list.len()).sum::<usize>(), 4);
    }

    #[test]
    fn a_search_covers_name_path_and_group_and_drops_empty_groups() {
        let tools = vec![tool("Compress", Some("Image")), tool("Merge", Some("PDF"))];
        let grouping = grouped(&tools);

        assert_eq!(group_tools(&tools, "compress", &grouping, false).len(), 1);
        // By path: every tool's path carries its name here.
        assert_eq!(
            group_tools(&tools, "/api/v1/tools/merge", &grouping, false)[0].1,
            vec![1]
        );
        // By group: the whole group answers.
        assert_eq!(group_tools(&tools, "image", &grouping, false)[0].1, vec![0]);
        assert!(group_tools(&tools, "nothing like this", &grouping, false).is_empty());
    }

    #[test]
    fn tools_that_only_hand_back_a_job_number_stay_out_of_the_list() {
        let mut tools = vec![
            tool("Compress", Some("Image")),
            tool("Transcribe", Some("Audio")),
        ];
        tools[1].usable = spec::Usable::Asynchronous;
        let grouping = grouped(&tools);

        // Off by default: on the test service these are 52 of 232, and none of
        // them can become a working entry.
        let groups = group_tools(&tools, "", &grouping, false);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1, vec![0]);

        // But reachable, so nothing vanishes without a way back.
        assert_eq!(group_tools(&tools, "", &grouping, true).len(), 2);
    }

    #[test]
    fn switching_services_clears_the_last_ones_typed_settings() {
        // The bug: `service_settings`/`service_fields` are keyed by index
        // into `service_tools`, and a freshly fetched service starts that
        // indexing over from zero. Left standing, a value typed for tool 3
        // of the old service would resurface as the value for tool 3 of the
        // new one -- a field the user never touched.
        let mut picked: rustc_hash::FxHashSet<usize> = [1, 3].into_iter().collect();
        let mut settings: rustc_hash::FxHashMap<usize, String> = rustc_hash::FxHashMap::default();
        settings.insert(3, "1920".into());
        let mut fields: rustc_hash::FxHashMap<(usize, String), String> =
            rustc_hash::FxHashMap::default();
        fields.insert((3, "width".into()), "1920".into());
        fields.insert((1, "format".into()), "png".into());

        clear_service_inputs(&mut picked, &mut settings, &mut fields);

        assert!(picked.is_empty());
        assert!(
            settings.is_empty(),
            "a value typed for the old service's tool must not survive a switch"
        );
        assert!(
            fields.is_empty(),
            "a field typed for the old service's tool must not survive a switch"
        );
    }

    #[test]
    fn a_failed_service_load_is_kept_as_an_error_not_swallowed_into_an_empty_list() {
        // The bug: the constructor used `service::load().unwrap_or_default()`
        // and hardcoded `service_error: None`, so a damaged `services.json`
        // looked exactly like an empty, healthy one -- and on `--tab
        // services` nothing else ever calls `reload_services` to notice.
        let (services, error) = services_from_load(Err(anyhow::anyhow!("kaputt")));
        assert!(services.is_empty());
        assert!(
            error.is_some(),
            "a load failure must be shown, not silently turned into an empty list"
        );

        let (services, error) = services_from_load(Ok(Vec::new()));
        assert!(services.is_empty());
        assert!(error.is_none(), "an empty list is not itself an error");
    }

    #[test]
    fn typed_settings_travel_as_the_types_the_description_declared() {
        let fields = vec![
            spec::Field {
                name: "width".into(),
                kind: spec::FieldKind::Number {
                    minimum: Some(1.0),
                    maximum: Some(8000.0),
                },
                required: false,
                description: None,
            },
            spec::Field {
                name: "grayscale".into(),
                kind: spec::FieldKind::Flag,
                required: false,
                description: None,
            },
            spec::Field {
                name: "format".into(),
                kind: spec::FieldKind::Choice(vec!["webp".into(), "png".into()]),
                required: false,
                description: None,
            },
            spec::Field {
                name: "untouched".into(),
                kind: spec::FieldKind::Text,
                required: false,
                description: None,
            },
        ];

        let json = settings_json(&fields, &|name| match name {
            "width" => Some("1920".into()),
            "grayscale" => Some("true".into()),
            "format" => Some("webp".into()),
            // Left empty on screen, and therefore not sent at all.
            _ => Some("   ".into()),
        })
        .expect("three fields were filled in");

        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["width"], 1920, "a number, not the string \"1920\"");
        // And a whole one at that: a field declared `integer` refuses 1920.0.
        assert!(!json.contains("1920.0"), "no decimal point: {json}");
        assert_eq!(value["grayscale"], true);
        assert_eq!(value["format"], "webp");
        assert!(
            value.get("untouched").is_none(),
            "an empty field means the service decides, not an empty string"
        );
    }

    #[test]
    fn a_whole_number_goes_out_whole_and_a_fraction_keeps_its_point() {
        let field = |name: &str| spec::Field {
            name: name.into(),
            kind: spec::FieldKind::Number {
                minimum: None,
                maximum: None,
            },
            required: false,
            description: None,
        };
        let fields = vec![field("targetSizeKb"), field("opacity")];

        let json = settings_json(&fields, &|name| match name {
            "targetSizeKb" => Some("1024".into()),
            _ => Some("0.75".into()),
        })
        .expect("both were filled in");

        // "1024.0" is what json!(f64) writes, and a service that declared the
        // field as `integer` may refuse it.
        assert!(json.contains("1024"), "{json}");
        assert!(!json.contains("1024.0"), "{json}");
        assert!(json.contains("0.75"), "a fraction stays one: {json}");
    }

    #[test]
    fn a_form_nobody_filled_in_sends_nothing_at_all() {
        let fields = vec![spec::Field {
            name: "width".into(),
            kind: spec::FieldKind::Text,
            required: false,
            description: None,
        }];
        assert_eq!(settings_json(&fields, &|_| None), None);
    }

    #[test]
    fn an_error_that_leaves_the_user_guessing_is_explained() {
        let tr = &i18n::DE;

        // The wordings these really arrive in, from four different layers.
        for (message, expected) in [
            ("Zugriff verweigert. (os error 5)", tr.why_access_denied),
            ("Access is denied. (os error 5)", tr.why_access_denied),
            ("Der Dienst antwortete mit 401", tr.why_unauthorised),
            ("the service answered 403", tr.why_unauthorised),
            ("Die Beschreibung antwortete mit 404", tr.why_not_found),
            ("WinHttpSendRequest: timed out", tr.why_unreachable),
            (
                "\x1eKennung schon vergeben\x1fid already taken\x1d: x",
                tr.why_id_taken,
            ),
        ] {
            assert_eq!(
                advice_for(message, tr),
                Some(expected),
                "no advice for {message:?}"
            );
        }

        // And nothing invented for a message that speaks for itself.
        assert_eq!(advice_for("Datei zu gro\u{df}", tr), None);
    }

    #[test]
    fn the_documentation_of_a_tool_is_the_page_beside_its_description() {
        let tagged = tool("Compress", Some("Tools"));

        // The stored address is the document; a human wants its directory.
        assert_eq!(
            docs_url("http://host:1349/api/docs/openapi.json", &tagged),
            "http://host:1349/api/docs/#tag/tools/POST/api/v1/tools/compress"
        );
        // An address that is already a page stays one.
        assert_eq!(
            docs_url("http://host:1349/api/docs/", &tagged),
            "http://host:1349/api/docs/#tag/tools/POST/api/v1/tools/compress"
        );

        // Without a tag there is no anchor to build, and the front page of the
        // documentation is still the right place to land.
        let untagged = tool("Compress", None);
        assert_eq!(
            docs_url("http://host:1349/api/docs/", &untagged),
            "http://host:1349/api/docs/"
        );
    }

    #[test]
    fn the_range_of_a_number_is_offered_as_far_as_it_is_known() {
        assert_eq!(number_hint(Some(1.0), Some(100.0)), "1 \u{2013} 100");
        assert_eq!(number_hint(Some(1.0), None), "\u{2265} 1");
        assert_eq!(number_hint(None, Some(100.0)), "\u{2264} 100");
        assert_eq!(number_hint(None, None), "");
    }

    #[test]
    fn shortening_never_splits_a_character() {
        // Four characters, seven bytes: a byte-wise cut would panic here.
        assert_eq!(
            shorten("\u{e4}\u{f6}\u{fc}\u{df}", 2),
            "\u{e4}\u{f6}\u{2026}"
        );
        assert_eq!(shorten("  kurz  ", 40), "kurz");
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
            glyphs.restore,
            glyphs.explorer,
            glyphs.link,
            glyphs.theme_system,
            glyphs.theme_light,
            glyphs.theme_dark,
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

    /// A handful of rectangles the toolbar could plausibly hand a flag,
    /// including two that are wrong on purpose.
    fn flag_rects() -> Vec<egui::Rect> {
        [
            // What the button asks for at the default style, and at 150 %.
            (24.0f32, 16.0f32),
            (36.0, 24.0),
            // Odd heights, where a third of the height is not a whole pixel.
            (25.0, 17.0),
            // Degenerate on purpose: a flag that is taller than it is wide, and
            // one with no room at all. Neither should be able to paint outside
            // itself, because the geometry is the only thing that decides.
            (8.0, 40.0),
            (1.0, 1.0),
        ]
        .into_iter()
        .map(|(width, height)| {
            egui::Rect::from_min_size(egui::pos2(17.5, 4.25), egui::vec2(width, height))
        })
        .collect()
    }

    #[test]
    fn the_german_bands_fill_their_rectangle_and_nothing_beside_it() {
        // The flag is painted straight onto the toolbar, so a band that
        // overshoots by a pixel lands on the button beside it. Deriving the
        // bands from the rectangle is what keeps that from happening at any
        // font size, and this is the check that the derivation holds.
        for rect in flag_rects() {
            let bands = german_bands(rect);
            for band in bands {
                assert!(
                    rect.contains_rect(band),
                    "{band:?} leaves {rect:?}: the flag would paint over its neighbour"
                );
            }
            assert_eq!(bands[0].top(), rect.top(), "no gap above the black band");
            assert_eq!(bands[2].bottom(), rect.bottom(), "none below the gold one");
            assert_eq!(bands[0].bottom(), bands[1].top(), "no seam black to red");
            assert_eq!(bands[1].bottom(), bands[2].top(), "none red to gold");
        }
    }

    #[test]
    fn the_union_jack_stays_inside_its_rectangle_however_thick_its_bars_are() {
        for rect in flag_rects() {
            for [from, to] in union_diagonals(rect) {
                assert!(rect.contains(from) && rect.contains(to), "{rect:?}");
            }
            // Every thickness the painter can ask for, and then some: a bar
            // wider than the flag has to be clamped rather than spill out.
            for width in [0.0f32, 1.5, rect.height() / 5.0, rect.height(), 500.0] {
                for bar in union_bars(rect, width) {
                    assert!(
                        rect.contains_rect(bar),
                        "a {width}-point bar leaves {rect:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_union_jacks_crossbars_meet_in_the_middle() {
        // The upright and the crossbar are one cross, not two stripes that
        // happen to overlap: both are centred, so they cross at the centre
        // whatever shape the rectangle has.
        for rect in flag_rects() {
            let [upright, crossbar] = union_bars(rect, rect.height() / 5.0);
            assert!(upright.contains(rect.center()));
            assert!(crossbar.contains(rect.center()));
            assert_eq!(upright.center().x, rect.center().x);
            assert_eq!(crossbar.center().y, rect.center().y);
        }
    }

    #[test]
    fn each_theme_state_has_its_own_picture_and_its_own_sentence() {
        // The button says everything it has to say through those two, so a
        // state sharing either with another would be a state the user cannot
        // tell apart from it.
        let glyphs = Glyphs::load();
        let states = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

        let pictures: Vec<char> = states
            .iter()
            .map(|state| theme_glyph(*state, &glyphs))
            .collect();
        assert_eq!(pictures.len(), 3);
        for (index, picture) in pictures.iter().enumerate() {
            assert!(
                !pictures[index + 1..].contains(picture),
                "two theme states share one glyph"
            );
        }

        for language in [&i18n::DE, &i18n::EN] {
            let tips: Vec<&str> = states
                .iter()
                .map(|state| theme_tip(*state, language))
                .collect();
            for (index, tip) in tips.iter().enumerate() {
                assert!(
                    !tips[index + 1..].contains(tip),
                    "two theme states share one tooltip: {tip}"
                );
            }
        }
    }

    #[test]
    fn every_settings_button_says_where_a_click_leads() {
        // Both buttons are pictures, and a picture that cannot be read is a
        // guess. The tooltip is the whole of the explanation, so it has to name
        // the other side by name -- in the language the window is speaking.
        assert!(i18n::DE.tip_language.contains("Englisch"));
        assert!(i18n::EN.tip_language.contains("German"));
        assert!(i18n::DE.tip_theme_system.contains("hell"));
        assert!(i18n::DE.tip_theme_light.contains("dunkel"));
        assert!(i18n::DE.tip_theme_dark.contains("System"));
        assert!(i18n::EN.tip_theme_system.contains("light"));
        assert!(i18n::EN.tip_theme_light.contains("dark"));
        assert!(i18n::EN.tip_theme_dark.contains("system"));
    }

    #[test]
    fn release_notes_lose_their_hashes_and_their_empty_stretches() {
        // What release-drafter actually produces, shortened.
        let notes = "## What's Changed\n\n\n* One thing by @someone\n* Another\n\n\n\n**Full Changelog**: https://example.invalid/compare\n";
        assert_eq!(
            plain_notes(notes),
            "What's Changed\n\n* One thing by @someone\n* Another\n\n**Full Changelog**: https://example.invalid/compare"
        );

        // A `#` inside a line is not a heading and stays put -- issue numbers
        // are written that way, and "fixes 42" is not the same sentence.
        assert_eq!(plain_notes("* fixes #42"), "* fixes #42");
        assert_eq!(plain_notes(""), "");
        assert_eq!(plain_notes("   \n\n  \n"), "");
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

    /// A shared ProgID's `from_ext` names whichever extension the scan
    /// reached it through first, which is not necessarily the one on screen.
    /// Regression guard: `.jpg` scanned first must not steal a `.png` entry
    /// when the user is looking at `.png`.
    #[test]
    fn a_shared_progid_uses_the_extension_on_screen_not_the_stale_one() {
        let mut entries = synthetic::scan_result(1).entries;
        entries[0].category = Category::ProgId {
            prog_id: "picviewer".into(),
            // Stale on purpose: the dedup in `scan::scan` keeps whichever
            // extension's source was processed first, which need not be the
            // one the user has selected now.
            from_ext: ".jpg".into(),
        };

        assert_eq!(
            category_for_new_entry(Some(&entries[0]), Tab::FileTypes, Some(".png"), None),
            Category::ExtAssoc(".png".into()),
            "the extension selected in the tree wins over the entry's stale from_ext"
        );

        // Without a selected extension to prefer, the stale `from_ext` is
        // still the best answer available.
        assert_eq!(
            category_for_new_entry(Some(&entries[0]), Tab::FileTypes, None, None),
            Category::ExtAssoc(".jpg".into())
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

        // And the other direction, because the console depends on it: there
        // the wording carries both halves, marked, and comes out in whichever
        // language is asked for.
        for fault in &fav {
            let marked = fault.marked();
            for language in [Language::German, Language::English] {
                let text = crate::bilingual::pick(&marked, language);
                assert!(!text.is_empty(), "{fault:?} has no wording");
                assert!(
                    !text.contains(crate::bilingual::is_marker),
                    "{fault:?} still carries a marker: {text}"
                );
            }
            assert_ne!(
                crate::bilingual::pick(&marked, Language::German),
                crate::bilingual::pick(&marked, Language::English),
                "{fault:?} lost a language the console needs"
            );
        }
        for fault in &entry {
            let marked = fault.marked();
            assert_ne!(
                crate::bilingual::pick(&marked, Language::German),
                crate::bilingual::pick(&marked, Language::English),
                "{fault:?} lost a language the console needs"
            );
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
                packaged: false,
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
