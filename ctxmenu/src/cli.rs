//! Command line front end.
//!
//! Hand-rolled rather than `clap`: the argument surface is tiny, `main` has to
//! inspect `argv` before starting the GUI anyway (the elevated job mode from
//! ToDo 13.2), and the release binary has a 15 MB budget to keep.
//!
//! This is a diagnostic tool for development. The shipping user interface is
//! the GUI, which gets its bilingual strings in milestone 5.

use anyhow::{Context as _, Result, bail};

use crate::model::{Category, ContextEntry, EntryKind, ScanProgress, Scope};
use crate::registry::paths::RegTarget;
use crate::registry::scan::{self, ScanOptions};
use crate::registry::{backup, write};

pub enum Command {
    /// The product itself. `synthetic` fills the table with generated rows
    /// instead of scanning, for the performance target of milestone 4.
    Gui {
        synthetic: Option<usize>,
        /// Run this many measured frames, report and exit.
        bench: Option<usize>,
        /// Which tab to open on.
        tab: crate::app::Tab,
        /// Text to put in the search box before the first frame. Exists so a
        /// search can be photographed and checked, not only tried by hand.
        search: String,
        /// Extension to preselect in the file type tab.
        ext: Option<String>,
    },
    Scan(ScanArgs),
    /// Group every entry by the program behind it (milestone 8).
    Programs,
    /// Walk the full resolution chain for one extension (milestone 7).
    FileType(String),
    /// Apply one action to one key, through the full plan and elevation path.
    Apply {
        action: crate::registry::plan::Action,
        path: String,
        confirmed: bool,
    },
    Backups,
    Restore(String),
    Delete {
        path: String,
        confirmed: bool,
    },
    /// Create an entry of one's own in HKCU (milestone 10).
    Create(Box<crate::registry::create::NewEntry>),
    /// List what this tool created, from entries.json.
    Created,
    /// The tool box. Subcommands, because five of them as top level verbs
    /// would crowd out the ones that scan.
    Favourite(FavouriteCommand),
    Smoke,
    Help,
}

pub enum FavouriteCommand {
    List,
    Add(Box<crate::favourites::Favourite>),
    /// Write a favourite into the context menu somewhere.
    Place {
        id: String,
        category: Category,
    },
    Remove(String),
    /// Do what a click on the entry would do, but report to the console
    /// instead of a message box — the only way to measure this path in the
    /// test VM, which has no interactive desktop for a dialog.
    Run {
        id: String,
        file: String,
    },
}

pub struct ScanArgs {
    pub options: ScanOptions,
    pub json: bool,
    pub quiet: bool,
}

pub const HELP: &str = "\
ctxmenu — Windows Context Menu Manager

Verwendung / Usage:
  ctxmenu                   Fenster öffnen / open the window
  ctxmenu --tab <name>      Fenster auf einem Reiter oeffnen / open on a tab:
                            categories, filetypes, programs, favourites,
                            backups
  ctxmenu --search <text>   Fenster mit gesetzter Suche oeffnen /
                            open the window with the search box filled
  ctxmenu --ext .png        Fenster auf dem Dateityp-Reiter, Endung gewaehlt /
                            file type tab with that extension selected
  ctxmenu --synthetic <n> [--bench <frames>]
                            Fenster mit n erzeugten Zeilen, optional als
                            Messlauf / window with n generated rows,
                            optionally as a measured run
  ctxmenu scan [Optionen]   Einträge auflisten / list context menu entries
  ctxmenu programs          Nach Programm gruppieren / group by program
  ctxmenu filetype <ext>    Auflösungskette eines Dateityps /
                            resolution chain of one file type
  ctxmenu hide|show|shift-only|always-show <key> --yes
                            Merkmal setzen oder entfernen, mit Backup und
                            nötigenfalls Rechteerhöhung / set or clear a flag,
                            with a backup and elevation if needed
  ctxmenu backups           Backups auflisten / list backups
  ctxmenu restore <pfad>    Backup zurückspielen / restore a backup directory
  ctxmenu delete <key> --yes
                            Schlüssel sichern und löschen /
                            back up and delete a key
  ctxmenu create --category <name> --name <text> --command <zeile>
                 [--key <name>] [--icon <ref>] [--position top|bottom]
                 [--extended]
                            Eigenen Eintrag in HKCU anlegen /
                            create your own entry in HKCU
  ctxmenu created           Selbst angelegte Eintraege auflisten /
                            list entries created by this tool
  ctxmenu favourites        Favoriten auflisten / list favourites
  ctxmenu favourite add --name <text>
        --exe <pfad> [--args <zeile>]                  Programm / a program
        --url <adresse> [--mode clipboard|open]        Webtool ohne Endpunkt
        --endpoint <adresse> [--raw] [--field <name>]  Webtool mit Upload
        [--header \"Name: Wert\"] [--result save|open|report]
        [--suffix .min] [--json-path output.url] [--insecure]
  ctxmenu favourite place <id> --category <name> | --ext .png | --perceived image
                            Favorit ins Kontextmenue eintragen /
                            put a favourite into the context menu
  ctxmenu favourite remove <id>
  ctxmenu favourite run <id> <datei>
                            Ausfuehren wie ein Klick, Ausgabe auf der Konsole /
                            run as a click would, reporting on the console
  ctxmenu --smoke           Smoke-Test-Fenster / open the smoke test window
  ctxmenu --help            Diese Hilfe / this help

Optionen / Options:
  --category <name>   Nur eine Kategorie / single category only:
                      allfiles, allfilesystemobjects, directory,
                      directorybackground, folder, desktopbackground, drive
  --scope <name>      user | machine | machine32 | all
                      (Vorgabe / default: all)
  --all-types         Auch die Dateityp-Kette / walk the file type chain too
  --json              Ausgabe als JSON / emit JSON on stdout
  --quiet             Kein Fortschritt / suppress progress output
";

pub fn parse(args: impl Iterator<Item = String>) -> Result<Command> {
    let args: Vec<String> = args.collect();

    // No arguments means the product: a window, not a usage message.
    if args.is_empty() {
        return Ok(Command::Gui {
            synthetic: None,
            bench: None,
            tab: crate::app::Tab::Categories,
            search: String::new(),
            ext: None,
        });
    }

    match args[0].as_str() {
        "--help" | "-h" | "help" => return Ok(Command::Help),
        "--smoke" => return Ok(Command::Smoke),
        "--synthetic" | "--bench" | "--tab" | "--search" | "--ext" => {
            let mut synthetic = None;
            let mut bench = None;
            let mut tab = crate::app::Tab::Categories;
            let mut search = String::new();
            let mut ext = None;
            let mut rest = args.iter();

            while let Some(flag) = rest.next() {
                let value = rest
                    .next()
                    .with_context(|| format!("{flag} erwartet einen Wert / expects a value"))?;
                match flag.as_str() {
                    "--search" => search = value.clone(),
                    "--ext" => {
                        ext = Some(value.to_lowercase());
                        tab = crate::app::Tab::FileTypes;
                    }
                    "--tab" => {
                        tab = crate::app::Tab::from_slug(value).with_context(|| {
                            format!("Unbekannter Reiter / unknown tab: {value}")
                        })?;
                    }
                    "--synthetic" | "--bench" => {
                        let number = value
                            .parse::<usize>()
                            .with_context(|| format!("Keine Zahl / not a number: {value}"))?;
                        if flag == "--synthetic" {
                            synthetic = Some(number);
                        } else {
                            bench = Some(number);
                        }
                    }
                    other => bail!("Unbekannte Option / unknown option: {other}\n\n{HELP}"),
                }
            }

            return Ok(Command::Gui {
                synthetic,
                bench,
                tab,
                search,
                ext,
            });
        }
        "programs" => return Ok(Command::Programs),
        "filetype" => {
            let ext = args
                .get(1)
                .context("filetype erwartet eine Erweiterung / expects an extension")?;
            return Ok(Command::FileType(ext.clone()));
        }
        "hide" | "show" | "shift-only" | "always-show" => {
            use crate::registry::plan::Action;
            let action = match args[0].as_str() {
                "hide" => Action::Hide,
                "show" => Action::Show,
                "shift-only" => Action::ShiftOnly,
                _ => Action::AlwaysShow,
            };
            let path = args
                .get(1)
                .with_context(|| format!("{} erwartet einen Registry-Pfad", args[0]))?;
            return Ok(Command::Apply {
                action,
                path: path.clone(),
                confirmed: args.iter().any(|a| a == "--yes"),
            });
        }
        "backups" => return Ok(Command::Backups),
        "restore" => {
            let directory = args
                .get(1)
                .context("restore erwartet ein Verzeichnis / expects a directory")?;
            return Ok(Command::Restore(directory.clone()));
        }
        "delete" => {
            let path = args
                .get(1)
                .context("delete erwartet einen Registry-Pfad / expects a registry path")?;
            return Ok(Command::Delete {
                path: path.clone(),
                confirmed: args.iter().any(|a| a == "--yes"),
            });
        }
        "created" => return Ok(Command::Created),
        "favourites" | "favoriten" => {
            return Ok(Command::Favourite(FavouriteCommand::List));
        }
        "favourite" | "favorit" => {
            return parse_favourite(&args[1..]).map(Command::Favourite);
        }
        "create" => {
            use crate::registry::create::NewEntry;
            let mut entry = NewEntry {
                category: Category::Directory,
                key_name: String::new(),
                display_name: String::new(),
                command: String::new(),
                icon: None,
                position: None,
                extended: false,
            };

            let mut rest = args[1..].iter();
            while let Some(flag) = rest.next() {
                if flag == "--extended" {
                    entry.extended = true;
                    continue;
                }
                let value = rest
                    .next()
                    .with_context(|| format!("{flag} erwartet einen Wert / expects a value"))?;
                match flag.as_str() {
                    "--category" => {
                        entry.category = Category::from_slug(value).with_context(|| {
                            format!("Unbekannte Kategorie / unknown category: {value}")
                        })?;
                    }
                    "--name" => entry.display_name = value.clone(),
                    "--key" => entry.key_name = value.clone(),
                    "--command" => entry.command = value.clone(),
                    "--icon" => entry.icon = Some(value.clone()),
                    "--position" => {
                        entry.position = Some(match value.to_ascii_lowercase().as_str() {
                            "top" | "oben" => "Top".to_string(),
                            "bottom" | "unten" => "Bottom".to_string(),
                            other => other.to_string(),
                        });
                    }
                    other => bail!("Unbekannte Option / unknown option: {other}\n\n{HELP}"),
                }
            }

            if entry.key_name.trim().is_empty() {
                entry.key_name = crate::registry::create::suggest_key_name(&entry.display_name);
            }
            return Ok(Command::Create(Box::new(entry)));
        }
        "scan" => {}
        other => bail!("Unbekannter Befehl / unknown command: {other}\n\n{HELP}"),
    }

    let mut options = ScanOptions::default();
    let mut json = false;
    let mut quiet = false;
    let mut rest = args[1..].iter();

    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--quiet" => quiet = true,
            // Walks the file type chain for every curated extension — what
            // the window does, and the only way to measure its cost.
            "--all-types" => {
                options.file_types = crate::registry::filetypes::CURATED
                    .iter()
                    .map(|d| d.ext.to_string())
                    .collect();
            }
            "--category" => {
                let value = rest
                    .next()
                    .context("--category erwartet einen Namen / expects a name")?;
                let category = Category::from_slug(value)
                    .with_context(|| format!("Unbekannte Kategorie / unknown category: {value}"))?;
                options.categories = Some(vec![category]);
            }
            "--scope" => {
                let value = rest
                    .next()
                    .context("--scope erwartet einen Namen / expects a name")?;
                options.scopes =
                    if value.eq_ignore_ascii_case("all") {
                        Scope::ALL.to_vec()
                    } else {
                        vec![Scope::from_slug(value).with_context(|| {
                            format!("Unbekannter Scope / unknown scope: {value}")
                        })?]
                    };
            }
            other => bail!("Unbekannte Option / unknown option: {other}\n\n{HELP}"),
        }
    }

    Ok(Command::Scan(ScanArgs {
        options,
        json,
        quiet,
    }))
}

pub fn run_scan(args: ScanArgs) -> Result<()> {
    let started = std::time::Instant::now();

    // Progress goes to stderr so that --json keeps stdout parseable.
    let result = scan::scan(&args.options, |p: ScanProgress| {
        if !args.quiet {
            crate::errln!(
                "[{:>2}/{:>2}] {:<60} {:>4} Einträge",
                p.done,
                p.total,
                p.label,
                p.found
            );
        }
    });

    if args.json {
        crate::outln!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let elapsed = started.elapsed();
    let nested = count_all(&result.entries) - result.entries.len();
    crate::outln!();
    crate::outln!(
        "{} Einträge (+{nested} in Untermenüs) in {:.2} s ({} Scopes, {})",
        result.entries.len(),
        elapsed.as_secs_f32(),
        args.options.scopes.len(),
        match &args.options.categories {
            Some(c) => c.iter().map(|c| c.slug()).collect::<Vec<_>>().join(", "),
            None => "alle Kategorien".to_string(),
        }
    );
    crate::outln!();

    crate::outln!(
        "{:<7} {:<8} {:<22} {:<34} {:<7} Befehl / CLSID",
        "Scope",
        "Typ",
        "Schlüssel",
        "Anzeigename",
        "Flags"
    );
    let rule = "-".repeat(120);
    crate::outln!("{rule}");

    for entry in &result.entries {
        print_entry(entry, 0);
    }

    print_summary(&result.entries, &args.options.scopes);
    crate::outln!(
        "MUI-Cache: {} Treffer / {} Auflösungen, blockierte CLSIDs im System: {}",
        result.stats.mui_cache_hits,
        result.stats.mui_cache_misses,
        result.stats.blocked_clsids
    );
    Ok(())
}

pub fn run_programs() -> Result<()> {
    let started = std::time::Instant::now();
    let result = scan::scan(&ScanOptions::default(), |_| {});

    let mut names = crate::program::identity::NameResolver::new();
    let groups = crate::program::group::build(&result, &mut names);
    let elapsed = started.elapsed();

    let grouped: usize = groups.iter().map(|g| g.entry_count()).sum();
    crate::outln!(
        "{} Programme aus {} Einträgen in {:.2} s ({} ohne zuordenbares Programm)",
        groups.len(),
        result.entries.len(),
        elapsed.as_secs_f32(),
        result.entries.len() - grouped
    );
    let (hits, lookups) = names.stats();
    crate::outln!("Namens-Cache: {hits} Treffer / {lookups} Auflösungen");
    crate::outln!();

    for group in &groups {
        let marks = [
            group.is_system.then_some("System"),
            group.read_only.then_some("schreibgeschützt"),
            (!group.clsids.is_empty()).then_some("COM"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");

        crate::outln!(
            "{:>3}x  {:<42} {}",
            group.entry_count(),
            truncate(&group.display_name, 42),
            if marks.is_empty() {
                String::new()
            } else {
                format!("[{marks}]")
            }
        );
        crate::outln!("      {}", group.key);
        crate::outln!("      {}", group.locations.join("  "));
    }

    Ok(())
}

pub fn run_file_type(raw_ext: &str) -> Result<()> {
    use crate::registry::filetypes;

    let ext = filetypes::normalize_ext(raw_ext)
        .with_context(|| format!("Keine Erweiterung / not an extension: {raw_ext}"))?;

    let started = std::time::Instant::now();
    let result = scan::scan(
        &ScanOptions {
            file_types: vec![ext.clone()],
            ..ScanOptions::default()
        },
        |_| {},
    );

    let info = result
        .file_types
        .first()
        .context("Der Scanner hat den Dateityp nicht geliefert")?;
    let r = &info.resolution;

    crate::outln!("{}   Gruppe: {:?}", r.ext, info.group);
    crate::outln!("{}", "-".repeat(100));
    crate::outln!("registriert:        {}", r.registered);
    crate::outln!(
        "Nutzerwahl:         {}",
        r.user_choice.as_deref().unwrap_or("—")
    );
    crate::outln!(
        "Systemvorgabe:      {}",
        r.default_progid.as_deref().unwrap_or("—")
    );
    crate::outln!(
        "wirksame ProgID:    {}",
        r.effective_progid().unwrap_or("—")
    );
    crate::outln!(
        "PerceivedType:      {}",
        r.perceived_type
            .as_deref()
            .unwrap_or("— (Ebene 3 entfällt)")
    );
    crate::outln!("OpenWithProgids:    {}", r.open_with_progids.join(", "));
    crate::outln!();

    // Levels 1 and 2 are shared by every file type; showing them separately
    // is the honest presentation, because deleting one of those hits every
    // other file type too (ToDo 10.4).
    let inherited: Vec<&ContextEntry> = result
        .entries
        .iter()
        .filter(|e| {
            matches!(
                e.category,
                Category::AllFiles | Category::AllFilesystemObjects
            )
        })
        .collect();

    crate::outln!(
        "Ebenen 1-2, gelten fuer ALLE Dateien: {} Einträge",
        inherited.len()
    );
    for entry in inherited.iter().take(6) {
        crate::outln!(
            "    {:<7} {:<10} {}",
            entry.scope.label(),
            entry.kind.type_label(),
            truncate(&entry.display_name, 60)
        );
    }
    if inherited.len() > 6 {
        crate::outln!("    … und {} weitere", inherited.len() - 6);
    }
    crate::outln!();

    crate::outln!(
        "Ebenen 3-7, nur fuer {}: {} Einträge",
        r.ext,
        info.own_entry_count()
    );
    for &index in &info.entry_indices {
        let entry = &result.entries[index];
        crate::outln!(
            "    {:<7} {:<10} {:<34} {}",
            entry.scope.label(),
            entry.kind.type_label(),
            truncate(&entry.display_name, 34),
            level_label(&entry.category)
        );
    }

    crate::outln!();
    crate::outln!(
        "Summe fuer einen Rechtsklick auf eine {}-Datei: {} Einträge, ermittelt in {:.2} s",
        r.ext,
        inherited.len() + info.own_entry_count(),
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

/// Which level of the chain an entry came from.
fn level_label(category: &Category) -> String {
    match category {
        Category::AllFiles => "1 alle Dateien".into(),
        Category::AllFilesystemObjects => "2 alle Dateisystemobjekte".into(),
        Category::PerceivedType(t) => format!("3 wahrgenommener Typ ({t})"),
        Category::ExtAssoc(e) => format!("4 SystemFileAssociations\\{e}"),
        Category::ProgId { prog_id, .. } => format!("5/7 ProgID {prog_id}"),
        Category::ExtDirect(e) => format!("6 {e}\\shell"),
        // Not part of the chain at all: a stock, not a level.
        Category::CommandStore => "— Verbvorrat".into(),
        other => format!("{other:?}"),
    }
}

/// Applies one action, taking the same route the window does.
///
/// Deliberately the same code path rather than a shortcut: this is how the
/// elevation handover gets exercised on a machine with no toolchain, and a
/// separate simplified path would prove nothing about the real one.
pub fn run_apply(action: crate::registry::plan::Action, path: &str, confirmed: bool) -> Result<()> {
    use crate::registry::plan::{Operation, Plan};

    let target = RegTarget::parse(path)?;
    if !write::exists(&target) {
        bail!(
            "Schlüssel existiert nicht / key does not exist: {}",
            target.full_path()
        );
    }

    let plan = Plan::new(
        action.label(),
        vec![Operation {
            display_name: target.relative.clone(),
            target: target.clone(),
            action,
            clsid: None,
        }],
    );

    let (direct, elevated) = plan.partition();
    let needs_elevation = !elevated.is_empty();

    if !confirmed {
        crate::outln!("Würde ausführen / would apply: {}", plan.label);
        crate::outln!("  {}", target.full_path());
        crate::outln!(
            "  Rechteerhöhung nötig / elevation required: {}",
            if needs_elevation {
                "ja / yes"
            } else {
                "nein / no"
            }
        );
        crate::outln!("Zum Ausführen --yes anhängen / append --yes to execute.");
        return Ok(());
    }

    let mut report = crate::registry::plan::execute(&direct)?;
    if needs_elevation {
        crate::outln!("Starte erhöhten Vorgang / starting elevated run …");
        report.merge(crate::elevation::run_elevated(&elevated)?);
    }

    // From the unelevated side, after the child is done.
    crate::elevation::notify_shell();

    crate::outln!(
        "{} erfolgreich / succeeded, {} fehlgeschlagen / failed",
        report.succeeded(),
        report.failed()
    );
    if let Some(directory) = &report.backup_directory {
        crate::outln!("Backup: {directory}");
    }
    for result in &report.results {
        match &result.error {
            None => crate::outln!("  ok   {}", result.registry_path),
            Some(error) => crate::outln!("  FEHL {}  —  {error}", result.registry_path),
        }
    }

    Ok(())
}

/// Creates an entry and tells the shell about it.
pub fn run_create(entry: &crate::registry::create::NewEntry) -> Result<()> {
    use crate::registry::create;

    for problem in create::check(entry) {
        // The console has no language setting, so it gets both halves.
        let text = problem.message();
        if problem.is_error() {
            crate::errln!("Fehler / error: {text}");
        } else {
            crate::errln!("Warnung / warning: {text}");
        }
    }

    let target = create::create(entry)?;
    // Without this the key is there and the running Explorer still shows the
    // old menu -- which looks exactly like a failed write.
    crate::elevation::notify_shell();

    crate::outln!("Angelegt / created: {}", target.full_path());
    crate::outln!("  {} -> {}", entry.display_name, entry.command);
    crate::outln!("  entries.json: {}", create::entries_path()?.display());
    Ok(())
}

/// Lists what this tool created, as recorded in `entries.json`.
pub fn run_created() -> Result<()> {
    let recorded = crate::registry::create::recorded()?;
    if recorded.is_empty() {
        crate::outln!("Nichts angelegt / nothing created yet");
        return Ok(());
    }

    for entry in &recorded {
        let location = entry
            .target()
            .map(|t| t.full_path())
            .unwrap_or_else(|_| "?".into());
        crate::outln!("{:<28} {}", entry.display_name, location);
        crate::outln!("    {}", entry.command);
    }
    Ok(())
}

/// `ctxmenu favourite <was> …`
fn parse_favourite(args: &[String]) -> Result<FavouriteCommand> {
    use crate::favourites::{
        Favourite, ResultAction, ResultSource, Tool, Upload, UploadBody, WebMode, WebTool,
    };

    let what = args
        .first()
        .map(String::as_str)
        .context("favourite erwartet list, add, place, remove oder run")?;

    match what {
        "list" => Ok(FavouriteCommand::List),

        "remove" => {
            let id = args.get(1).context("remove erwartet eine Kennung")?;
            Ok(FavouriteCommand::Remove(id.clone()))
        }

        "run" => {
            let (Some(id), Some(file)) = (args.get(1), args.get(2)) else {
                bail!("run erwartet eine Kennung und eine Datei / expects an id and a file");
            };
            Ok(FavouriteCommand::Run {
                id: id.clone(),
                file: file.clone(),
            })
        }

        "place" => {
            let id = args.get(1).context("place erwartet eine Kennung")?.clone();
            let mut category = None;
            let mut rest = args[2..].iter();

            while let Some(flag) = rest.next() {
                let value = rest
                    .next()
                    .with_context(|| format!("{flag} erwartet einen Wert"))?;
                category = Some(match flag.as_str() {
                    "--category" => Category::from_slug(value).with_context(|| {
                        format!("Unbekannte Kategorie / unknown category: {value}")
                    })?,
                    "--ext" => Category::ExtAssoc(value.clone()),
                    "--perceived" => Category::PerceivedType(value.clone()),
                    other => bail!("Unbekannte Option / unknown option: {other}"),
                });
            }

            Ok(FavouriteCommand::Place {
                id,
                category: category.context("place erwartet --category, --ext oder --perceived")?,
            })
        }

        "add" => {
            let mut name = String::new();
            let mut exe: Option<String> = None;
            let mut program_args = String::new();
            let mut url: Option<String> = None;
            let mut mode = "clipboard".to_string();
            let mut endpoint: Option<String> = None;
            let mut field = "file".to_string();
            let mut raw = false;
            let mut insecure = false;
            let mut headers: Vec<crate::favourites::Header> = Vec::new();
            let mut result = "report".to_string();
            let mut suffix = ".neu".to_string();
            let mut json_path: Option<String> = None;
            let mut icon: Option<String> = None;

            let mut rest = args[1..].iter();
            while let Some(flag) = rest.next() {
                match flag.as_str() {
                    "--raw" => {
                        raw = true;
                        continue;
                    }
                    "--insecure" => {
                        insecure = true;
                        continue;
                    }
                    _ => {}
                }

                let value = rest
                    .next()
                    .with_context(|| format!("{flag} erwartet einen Wert"))?;
                match flag.as_str() {
                    "--name" => name = value.clone(),
                    "--exe" => exe = Some(value.clone()),
                    "--args" => program_args = value.clone(),
                    "--url" => url = Some(value.clone()),
                    "--mode" => mode = value.to_lowercase(),
                    "--endpoint" => endpoint = Some(value.clone()),
                    "--field" => field = value.clone(),
                    "--icon" => icon = Some(value.clone()),
                    "--result" => result = value.to_lowercase(),
                    "--suffix" => suffix = value.clone(),
                    "--json-path" => json_path = Some(value.clone()),
                    "--header" => {
                        let (key, val) = value
                            .split_once(':')
                            .context("--header erwartet \"Name: Wert\"")?;
                        headers.push(crate::favourites::Header {
                            name: key.trim().to_string(),
                            value: val.trim().to_string(),
                        });
                    }
                    other => bail!("Unbekannte Option / unknown option: {other}"),
                }
            }

            let source = match json_path {
                Some(path) => ResultSource::Json { path },
                None => ResultSource::Body,
            };

            let tool = match (exe, endpoint, url) {
                (Some(path), _, _) => Tool::Program {
                    path: std::path::PathBuf::from(path.trim().trim_matches('"')),
                    args: program_args,
                },
                (None, Some(endpoint), _) => Tool::Web(WebTool {
                    mode: WebMode::Upload(Upload {
                        endpoint,
                        method: "POST".into(),
                        body: if raw {
                            UploadBody::Raw
                        } else {
                            UploadBody::Multipart { field }
                        },
                        headers,
                        fields: Vec::new(),
                        result: match result.as_str() {
                            "save" => ResultAction::Save { source, suffix },
                            "open" => ResultAction::Open { source },
                            _ => ResultAction::Report,
                        },
                    }),
                    allow_insecure: insecure,
                    confirmed: false,
                }),
                (None, None, Some(url)) => Tool::Web(WebTool {
                    mode: match mode.as_str() {
                        "open" => WebMode::Open { url },
                        _ => WebMode::Clipboard { url },
                    },
                    allow_insecure: insecure,
                    confirmed: false,
                }),
                (None, None, None) => {
                    bail!("add erwartet --exe, --url oder --endpoint")
                }
            };

            Ok(FavouriteCommand::Add(Box::new(Favourite {
                id: String::new(),
                name,
                icon,
                note: None,
                tool,
            })))
        }

        other => bail!("Unbekannt / unknown: favourite {other}"),
    }
}

pub fn run_favourite(command: FavouriteCommand) -> Result<()> {
    use crate::favourites;

    match command {
        FavouriteCommand::List => {
            let list = favourites::load()?;
            if list.is_empty() {
                crate::outln!("Keine Favoriten / no favourites");
                crate::outln!("  {}", favourites::path()?.display());
                return Ok(());
            }

            for favourite in &list {
                crate::outln!("{:<20} {}", favourite.id, favourite.name);
                match &favourite.tool {
                    crate::favourites::Tool::Program { path, args } => {
                        crate::outln!("    Programm  {} {}", path.display(), args);
                    }
                    crate::favourites::Tool::Web(_) => {
                        crate::outln!(
                            "    Webtool   {}{}",
                            favourite.address().unwrap_or_default(),
                            if favourite.transfers_the_file() {
                                "   (sendet die Datei)"
                            } else {
                                ""
                            }
                        );
                    }
                }
            }
            Ok(())
        }

        FavouriteCommand::Add(favourite) => {
            for problem in favourite.problems() {
                crate::errln!("Hinweis / note: {problem}");
            }
            let id = favourites::add(*favourite)?;
            crate::outln!("Angelegt / created: {id}");
            Ok(())
        }

        FavouriteCommand::Remove(id) => {
            favourites::remove(&id)?;
            crate::outln!("Entfernt / removed: {id}");
            Ok(())
        }

        FavouriteCommand::Place { id, category } => {
            let favourite = favourites::find(&id)?;
            let exe = std::env::current_exe().context("Eigenen Pfad nicht ermittelbar")?;
            let entry = favourite.entry(category, &exe);

            for problem in crate::registry::create::check(&entry) {
                crate::errln!("Hinweis / note: {}", problem.message());
            }

            let target = crate::registry::create::create(&entry)?;
            crate::elevation::notify_shell();
            crate::outln!("Angelegt / created: {}", target.full_path());
            crate::outln!("  {}", entry.command);
            Ok(())
        }

        FavouriteCommand::Run { id, file } => {
            let message = crate::webtool::run(&id, std::path::Path::new(&file))?;
            crate::outln!("{message}");
            Ok(())
        }
    }
}

pub fn run_backups() -> Result<()> {
    let backups = backup::list()?;
    if backups.is_empty() {
        // `display()`, not `{:?}`: the debug form doubles every backslash, so
        // the path it prints is one nobody can paste anywhere.
        crate::outln!(
            "Keine Backups unter {} / no backups yet",
            backup::root_dir()?.display()
        );
        return Ok(());
    }

    for (directory, manifest) in &backups {
        crate::outln!(
            "{}  {:<20} {} Schlüssel{}",
            manifest.created_at.format("%Y-%m-%d %H:%M:%S"),
            manifest.action,
            manifest.entries.len(),
            if manifest.missing.is_empty() {
                String::new()
            } else {
                format!(", {} fehlten beim Export", manifest.missing.len())
            }
        );
        crate::outln!("    {}", directory.display());
        for entry in &manifest.entries {
            crate::outln!("      {}", entry.registry_path);
        }
        for note in &manifest.notes {
            crate::outln!("      ! {note}");
        }
    }
    Ok(())
}

pub fn run_restore(directory: &str) -> Result<()> {
    let path = std::path::Path::new(directory);
    let restored = backup::restore(path)?;
    crate::outln!("{restored} Datei(en) zurückgespielt / restored from {directory}");
    crate::outln!(
        "Hinweis: reg import fügt hinzu und überschreibt, entfernt aber nichts. / \
         note: reg import adds and overwrites, it never removes."
    );
    Ok(())
}

/// Backs a key up and then deletes it.
///
/// Requires `--yes`. There is deliberately no interactive prompt: this path
/// exists to make milestone 3 reproducible by hand, and the real confirmation
/// dialog belongs in the GUI.
pub fn run_delete(path: &str, confirmed: bool) -> Result<()> {
    let target = RegTarget::parse(path)?;

    if !write::exists(&target) {
        bail!(
            "Schlüssel existiert nicht / key does not exist: {}",
            target.full_path()
        );
    }

    if !confirmed {
        crate::outln!("Würde sichern und löschen / would back up and delete:");
        crate::outln!("  {}", target.full_path());
        crate::outln!("Zum Ausführen --yes anhängen / append --yes to execute.");
        return Ok(());
    }

    let token = backup::export_targets("delete", std::slice::from_ref(&target))?;
    crate::outln!("Backup: {}", token.directory().display());

    write::delete_tree(&target, &token)?;
    crate::outln!("Gelöscht / deleted: {}", target.full_path());
    crate::outln!(
        "Zurückholen mit / restore with: ctxmenu restore \"{}\"",
        token.directory().display()
    );
    Ok(())
}

/// Counts entries including cascading submenu children.
fn count_all(entries: &[ContextEntry]) -> usize {
    entries
        .iter()
        .map(|e| match &e.kind {
            EntryKind::Verb { sub_commands, .. } => 1 + count_all(sub_commands),
            EntryKind::ShellEx { .. } => 1,
        })
        .sum()
}

fn print_entry(entry: &ContextEntry, indent: usize) {
    let pad = "  ".repeat(indent);
    let detail = match &entry.kind {
        EntryKind::Verb { command, .. } => command.clone().unwrap_or_else(|| "—".into()),
        EntryKind::ShellEx { clsid, .. } => clsid.clone(),
    };

    crate::outln!(
        "{:<7} {:<8} {pad}{:<22} {:<34} {:<7} {}",
        entry.scope.label(),
        entry.kind.type_label(),
        truncate(&entry.key_name, 22 - indent * 2),
        truncate(&entry.display_name, 34),
        flags(entry),
        truncate(&detail, 40),
    );

    if let EntryKind::Verb { sub_commands, .. } = &entry.kind {
        for child in sub_commands {
            print_entry(child, indent + 1);
        }
    }
}

/// Compact flag column: read-only, hidden, shift-only, forced position.
fn flags(entry: &ContextEntry) -> String {
    let mut out = String::new();
    if entry.read_only {
        out.push_str("ro ");
    }
    if entry.hidden {
        out.push_str("hid ");
    }
    if entry.extended {
        out.push_str("shift ");
    }
    if let Some(position) = &entry.position {
        out.push_str(&position.chars().take(3).collect::<String>());
    }
    out.trim_end().to_string()
}

fn print_summary(entries: &[ContextEntry], requested: &[Scope]) {
    use std::collections::BTreeMap;

    let mut per_scope: BTreeMap<&str, usize> = BTreeMap::new();
    let mut per_category: BTreeMap<String, usize> = BTreeMap::new();
    let mut shellex = 0;

    // Seed every requested scope so a scope with no hits shows up as 0 rather
    // than vanishing — "nothing found" and "not looked at" must stay
    // distinguishable.
    for scope in requested {
        per_scope.entry(scope.label()).or_default();
    }

    for entry in entries {
        *per_scope.entry(entry.scope.label()).or_default() += 1;
        *per_category.entry(entry.category.slug()).or_default() += 1;
        if matches!(entry.kind, EntryKind::ShellEx { .. }) {
            shellex += 1;
        }
    }

    crate::outln!();
    crate::outln!("Nach Scope:     {per_scope:?}");
    crate::outln!("Nach Kategorie: {per_category:?}");
    crate::outln!(
        "Davon COM-Handler: {shellex}, statische Verben: {}",
        entries.len() - shellex
    );
}

/// Truncates on character boundaries so umlauts do not split mid-encoding.
fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    let mut out: String = value.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Command> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_arguments_opens_the_window() {
        // This is a GUI application that happens to have a command line, not
        // the other way round: a bare double-click must show the product.
        assert!(matches!(
            parse_args(&[]).unwrap(),
            Command::Gui {
                synthetic: None,
                bench: None,
                ..
            }
        ));
        assert!(matches!(parse_args(&["--help"]).unwrap(), Command::Help));
    }

    #[test]
    fn the_synthetic_row_count_is_parsed() {
        assert!(matches!(
            parse_args(&["--synthetic", "2000"]).unwrap(),
            Command::Gui {
                synthetic: Some(2000),
                bench: None,
                ..
            }
        ));
        assert!(matches!(
            parse_args(&["--synthetic", "2000", "--bench", "600"]).unwrap(),
            Command::Gui {
                synthetic: Some(2000),
                bench: Some(600),
                ..
            }
        ));
        assert!(parse_args(&["--synthetic"]).is_err());
        assert!(parse_args(&["--synthetic", "viele"]).is_err());
    }

    #[test]
    fn scan_defaults_to_every_scope_and_category() {
        let Command::Scan(args) = parse_args(&["scan"]).unwrap() else {
            panic!("expected a scan command");
        };
        assert_eq!(args.options.scopes.len(), 3);
        assert!(args.options.categories.is_none());
        assert!(!args.json);
    }

    #[test]
    fn category_and_scope_are_parsed() {
        let Command::Scan(args) = parse_args(&[
            "scan",
            "--category",
            "directory",
            "--scope",
            "machine32",
            "--json",
        ])
        .unwrap() else {
            panic!("expected a scan command");
        };
        assert_eq!(args.options.categories, Some(vec![Category::Directory]));
        assert_eq!(args.options.scopes, vec![Scope::Machine32]);
        assert!(args.json);
    }

    #[test]
    fn unknown_values_are_rejected_rather_than_ignored() {
        assert!(parse_args(&["scan", "--category", "nonsense"]).is_err());
        assert!(parse_args(&["scan", "--scope", "nonsense"]).is_err());
        assert!(parse_args(&["scan", "--category"]).is_err());
        assert!(parse_args(&["nonsense"]).is_err());
    }

    #[test]
    fn a_new_entry_is_assembled_from_the_flags() {
        let Command::Create(entry) = parse_args(&[
            "create",
            "--category",
            "directorybackground",
            "--name",
            "Hier öffnen",
            "--command",
            r#""C:\Windows\notepad.exe" "%V""#,
            "--position",
            "oben",
            "--extended",
        ])
        .unwrap() else {
            panic!("expected a create command");
        };

        assert_eq!(entry.category, Category::DirectoryBackground);
        assert_eq!(entry.display_name, "Hier öffnen");
        assert_eq!(entry.position.as_deref(), Some("Top"));
        assert!(entry.extended);
        // Not given, so derived — otherwise the key would be nameless.
        assert_eq!(entry.key_name, "ctxmenu_Hier_öffnen");
        // A flag taking no value must not swallow the next one.
        assert!(entry.command.contains("%V"));
    }

    #[test]
    fn truncation_keeps_character_boundaries() {
        assert_eq!(truncate("kurz", 10), "kurz");
        assert_eq!(truncate("äöüäöüäöü", 4), "äöü…");
        assert_eq!(truncate("abcdef", 6), "abcdef");
    }
}
