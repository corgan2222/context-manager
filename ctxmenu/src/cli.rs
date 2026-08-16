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
    /// The product itself. Everything the flags had to say about how it should
    /// open travels in one [`crate::app::Start`].
    Gui(crate::app::Start),
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
    /// Back up every place this tool touches, in one go.
    BackupAll,
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
    /// Which build this is — the question every bug report starts with.
    Version,
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

/// The usage text, in the language this machine speaks.
///
/// The one text that could not take the markers of [`crate::bilingual`]: it is
/// a table, and a table cut in half keeps the width of the half that was
/// thrown away. Two texts instead, held side by side by a test that compares
/// the commands they list.
pub fn help() -> &'static str {
    match crate::bilingual::language() {
        crate::settings::Language::German => HELP_DE,
        crate::settings::Language::English => HELP_EN,
    }
}

const HELP_DE: &str = "\
ctxmenu — Windows Context Menu Manager

Verwendung:
  ctxmenu                   Fenster öffnen
  ctxmenu --tab <name>      Fenster auf einem Reiter öffnen:
                            categories, filetypes, programs, favourites,
                            services, backups
  ctxmenu --search <text>   Fenster mit gesetzter Suche öffnen
  ctxmenu --ext .png        Fenster auf dem Dateityp-Reiter, Endung gewählt
  ctxmenu --lang de|en      Fenster in dieser Sprache öffnen; die gespeicherte
                            Einstellung bleibt, wie sie ist
  ctxmenu --window 1600x1000
                            Fenster in dieser Größe (Bildpunkte) auf dem linken
                            Bildschirm öffnen; mindestens 900x600
  ctxmenu --synthetic <n> [--bench <frames>]
                            Fenster mit n erzeugten Zeilen, optional als
                            Messlauf
  ctxmenu scan [Optionen]   Einträge auflisten
  ctxmenu programs          Nach Programm gruppieren
  ctxmenu filetype <ext>    Auflösungskette eines Dateityps
  ctxmenu hide|show|shift-only|always-show <key> --yes
                            Merkmal setzen oder entfernen, mit Backup und
                            nötigenfalls Rechteerhöhung
  ctxmenu backups           Backups auflisten
  ctxmenu backup-all        Alles sichern, was dieses Werkzeug anfasst
  ctxmenu restore <pfad>    Backup zurückspielen
  ctxmenu delete <key> --yes
                            Schlüssel sichern und löschen
  ctxmenu create --category <name> | --ext .png | --perceived image
                 --name <text> --command <zeile>
                 [--key <name>] [--icon <ref>] [--position top|bottom]
                 [--extended]
                            Eigenen Eintrag in HKCU anlegen
                 --sub \"<text>|<zeile>\" [--sub-icon <ref>] ...
                            Statt --command: Untermenü, ein --sub je
                            Untereintrag, getrennt am ersten senkrechten
                            Strich; --sub-icon gilt dem davorstehenden --sub
  ctxmenu created           Selbst angelegte Einträge auflisten
  ctxmenu favourites        Favoriten auflisten
  ctxmenu favourite add --name <text>
        --exe <pfad> [--args <zeile>]                  Programm
        --url <adresse> [--mode clipboard|open]        Webtool ohne Endpunkt
        --endpoint <adresse> [--raw] [--field <name>]  Webtool mit Upload
        [--header \"Name: Wert\"] [--result save|open|report]
        [--suffix .min] [--json-path output.url] [--insecure]
  ctxmenu favourite place <id> --category <name> | --ext .png | --perceived image
                            Favorit ins Kontextmenü eintragen
  ctxmenu favourite remove <id>
  ctxmenu favourite run <id> <datei>
                            Ausführen wie ein Klick, Ausgabe auf der Konsole
  ctxmenu --theme-probe     Systemthema einmal umschalten und melden, ob das
                            Fenster folgt; setzt die Einstellung danach zurück
  ctxmenu --smoke           Smoke-Test-Fenster
  ctxmenu --version         Fassung nennen
  ctxmenu --help            Diese Hilfe

Optionen:
  --category <name>   Nur eine Kategorie:
                      allfiles, allfilesystemobjects, directory,
                      directorybackground, folder, desktopbackground, drive
  --scope <name>      user | machine | machine32 | all
                      (Vorgabe: all)
  --all-types         Auch die Dateityp-Kette, für die vorgegebene Liste und
                      eigene Endungen
  --every-type        Statt dessen jede registrierte Endung dieses Rechners
  --json              Ausgabe als JSON
  --quiet             Kein Fortschritt
";

const HELP_EN: &str = "\
ctxmenu — Windows Context Menu Manager

Usage:
  ctxmenu                   open the window
  ctxmenu --tab <name>      open the window on a tab:
                            categories, filetypes, programs, favourites,
                            services, backups
  ctxmenu --search <text>   open the window with the search box filled
  ctxmenu --ext .png        open the file type tab with that extension
                            selected
  ctxmenu --lang de|en      open the window in that language, leaving the
                            saved setting as it is
  ctxmenu --window 1600x1000
                            open the window at that size in pixels on the
                            leftmost screen; at least 900x600
  ctxmenu --synthetic <n> [--bench <frames>]
                            window with n generated rows, optionally as a
                            measured run
  ctxmenu scan [options]    list context menu entries
  ctxmenu programs          group by program
  ctxmenu filetype <ext>    resolution chain of one file type
  ctxmenu hide|show|shift-only|always-show <key> --yes
                            set or clear a flag, with a backup and elevation
                            if needed
  ctxmenu backups           list backups
  ctxmenu backup-all        back up every place this tool touches
  ctxmenu restore <path>    restore a backup directory
  ctxmenu delete <key> --yes
                            back up and delete a key
  ctxmenu create --category <name> | --ext .png | --perceived image
                 --name <text> --command <line>
                 [--key <name>] [--icon <ref>] [--position top|bottom]
                 [--extended]
                            create your own entry in HKCU
                 --sub \"<text>|<line>\" [--sub-icon <ref>] ...
                            instead of --command: a submenu, one --sub per
                            child, split at the first vertical bar; --sub-icon
                            applies to the --sub before it
  ctxmenu created           list entries created by this tool
  ctxmenu favourites        list favourites
  ctxmenu favourite add --name <text>
        --exe <path> [--args <line>]                   a program
        --url <address> [--mode clipboard|open]        web tool, no endpoint
        --endpoint <address> [--raw] [--field <name>]  web tool with upload
        [--header \"Name: Value\"] [--result save|open|report]
        [--suffix .min] [--json-path output.url] [--insecure]
  ctxmenu favourite place <id> --category <name> | --ext .png | --perceived image
                            put a favourite into the context menu
  ctxmenu favourite remove <id>
  ctxmenu favourite run <id> <file>
                            run as a click would, reporting on the console
  ctxmenu --theme-probe     flip the system theme once, report whether the
                            window followed, then restore it
  ctxmenu --smoke           open the smoke test window
  ctxmenu --version         print the version
  ctxmenu --help            this help

Options:
  --category <name>   single category only:
                      allfiles, allfilesystemobjects, directory,
                      directorybackground, folder, desktopbackground, drive
  --scope <name>      user | machine | machine32 | all
                      (default: all)
  --all-types         walk the file type chain for the curated list plus
                      one's own extensions
  --every-type        instead: every extension registered on this machine
  --json              emit JSON on stdout
  --quiet             suppress progress output
";

pub fn parse(args: impl Iterator<Item = String>) -> Result<Command> {
    let args: Vec<String> = args.collect();

    // No arguments means the product: a window, not a usage message.
    if args.is_empty() {
        return Ok(Command::Gui(crate::app::Start::default()));
    }

    match args[0].as_str() {
        "--help" | "-h" | "help" => return Ok(Command::Help),
        "--version" | "-V" | "version" => return Ok(Command::Version),
        "--smoke" => return Ok(Command::Smoke),
        // Takes no value, so it cannot join the loop below.
        "--theme-probe" => {
            return Ok(Command::Gui(crate::app::Start {
                theme_probe: true,
                ..Default::default()
            }));
        }
        "--synthetic" | "--bench" | "--tab" | "--search" | "--ext" | "--lang" | "--window" => {
            let mut start = crate::app::Start::default();
            let mut rest = args.iter();

            while let Some(flag) = rest.next() {
                let value = rest.next().with_context(|| {
                    format!("{flag} \x1eerwartet einen Wert\x1fexpects a value\x1d")
                })?;
                match flag.as_str() {
                    "--search" => start.search = value.clone(),
                    "--ext" => {
                        start.ext = Some(value.to_lowercase());
                        start.tab = crate::app::Tab::FileTypes;
                    }
                    "--tab" => {
                        start.tab = crate::app::Tab::from_slug(value).with_context(|| {
                            format!("\x1eUnbekannter Reiter\x1funknown tab\x1d: {value}")
                        })?;
                    }
                    "--lang" => {
                        start.language = Some(
                            crate::settings::Language::from_slug(value).with_context(|| {
                                format!("\x1eUnbekannte Sprache\x1funknown language\x1d: {value}")
                            })?,
                        );
                    }
                    "--window" => start.size = Some(parse_window_size(value)?),
                    "--synthetic" | "--bench" => {
                        let number = value.parse::<usize>().with_context(|| {
                            format!("\x1eKeine Zahl\x1fnot a number\x1d: {value}")
                        })?;
                        if flag == "--synthetic" {
                            start.synthetic = Some(number);
                        } else {
                            // A run over zero frames measures nothing, and
                            // the counter it would drive is unsigned: caught
                            // here first so it never even reaches that guard.
                            if number == 0 {
                                bail!(
                                    "\x1e--bench 0 misst nichts\x1f--bench 0 measures nothing\x1d"
                                );
                            }
                            start.bench = Some(number);
                        }
                    }
                    other => bail!(
                        "\x1eUnbekannte Option\x1funknown option\x1d: {other}\n\n{}",
                        help()
                    ),
                }
            }

            return Ok(Command::Gui(start));
        }
        "programs" => return Ok(Command::Programs),
        "filetype" => {
            let ext = args
                .get(1)
                .context("\x1efiletype erwartet eine Erweiterung\x1fexpects an extension\x1d")?;
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
        "backup-all" => return Ok(Command::BackupAll),
        "restore" => {
            let directory = args
                .get(1)
                .context("\x1erestore erwartet ein Verzeichnis\x1fexpects a directory\x1d")?;
            return Ok(Command::Restore(directory.clone()));
        }
        "delete" => {
            let path = args.get(1).context(
                "\x1edelete erwartet einen Registry-Pfad\x1fexpects a registry path\x1d",
            )?;
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
            use crate::registry::create::{NewChild, NewEntry};
            let mut entry = NewEntry {
                category: Category::Directory,
                key_name: String::new(),
                display_name: String::new(),
                command: String::new(),
                icon: None,
                position: None,
                extended: false,
                children: Vec::new(),
            };

            let mut rest = args[1..].iter();
            while let Some(flag) = rest.next() {
                if flag == "--extended" {
                    entry.extended = true;
                    continue;
                }
                let value = rest.next().with_context(|| {
                    format!("{flag} \x1eerwartet einen Wert\x1fexpects a value\x1d")
                })?;
                match flag.as_str() {
                    "--category" => {
                        entry.category = Category::from_slug(value).with_context(|| {
                            format!("\x1eUnbekannte Kategorie\x1funknown category\x1d: {value}")
                        })?;
                    }
                    // The same two ways `favourite place` has always offered.
                    // Without them an entry of one's own could be written to a
                    // base category and nowhere else from the command line,
                    // while the window and the favourites could both do it —
                    // which is where a submenu for `.png` ran aground.
                    "--ext" => entry.category = Category::ExtAssoc(value.clone()),
                    "--perceived" => entry.category = Category::PerceivedType(value.clone()),
                    "--name" => entry.display_name = value.clone(),
                    "--key" => entry.key_name = value.clone(),
                    "--command" => entry.command = value.clone(),
                    "--icon" => entry.icon = Some(value.clone()),
                    // Split at the *first* bar, so a command may contain more
                    // of them: `--sub "Auflisten|cmd /c dir | more"` is a
                    // display name and a pipeline, not three fields.
                    "--sub" => {
                        let (name, command) = value.split_once('|').with_context(|| {
                            format!(
                                "\x1e--sub erwartet \"Anzeigename|Befehl\"\
                                 \x1f--sub expects \"display name|command\"\x1d: {value}"
                            )
                        })?;
                        entry.children.push(NewChild {
                            // Filled in below, once the order is known.
                            key_name: String::new(),
                            display_name: name.trim().to_string(),
                            command: command.trim().to_string(),
                            icon: None,
                        });
                    }
                    // Belongs to the `--sub` in front of it. A flag of its own
                    // rather than a third field, because a command line may
                    // contain bars and an icon path may contain anything.
                    "--sub-icon" => {
                        let child = entry.children.last_mut().with_context(|| {
                            "\x1e--sub-icon gehört zu einem vorangehenden --sub\
                             \x1f--sub-icon belongs to a preceding --sub\x1d"
                                .to_string()
                        })?;
                        child.icon = Some(value.clone());
                    }
                    "--position" => {
                        entry.position = Some(match value.to_ascii_lowercase().as_str() {
                            "top" | "oben" => "Top".to_string(),
                            "bottom" | "unten" => "Bottom".to_string(),
                            other => other.to_string(),
                        });
                    }
                    other => bail!(
                        "\x1eUnbekannte Option\x1funknown option\x1d: {other}\n\n{}",
                        help()
                    ),
                }
            }

            if entry.key_name.trim().is_empty() {
                entry.key_name = crate::registry::create::suggest_key_name(&entry.display_name);
            }
            // The child key names carry the order, so they are derived from
            // the order the flags came in rather than taken from the user.
            for (index, child) in entry.children.iter_mut().enumerate() {
                child.key_name =
                    crate::registry::create::suggest_child_key_name(index, &child.display_name);
            }
            return Ok(Command::Create(Box::new(entry)));
        }
        "scan" => {}
        other => bail!(
            "\x1eUnbekannter Befehl\x1funknown command\x1d: {other}\n\n{}",
            help()
        ),
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
                // The curated list plus whatever the user added in the window,
                // which is exactly what the window itself walks.
                let settings = crate::settings::Settings::load_or_default(
                    crate::settings::Language::default(),
                );
                options.file_types =
                    crate::registry::filetypes::wanted(&settings.custom_extensions);
            }
            // Every extension the machine has, not the curated selection.
            // Its own flag rather than a wider `--all-types`, because the
            // difference is 98 against about 1900 and that is a decision, not
            // a detail (ToDo 10.3).
            "--every-type" => {
                options.file_types = crate::registry::filetypes::installed();
            }
            "--category" => {
                let value = rest
                    .next()
                    .context("\x1e--category erwartet einen Namen\x1fexpects a name\x1d")?;
                let category = Category::from_slug(value).with_context(|| {
                    format!("\x1eUnbekannte Kategorie\x1funknown category\x1d: {value}")
                })?;
                options.categories = Some(vec![category]);
            }
            "--scope" => {
                let value = rest
                    .next()
                    .context("\x1e--scope erwartet einen Namen\x1fexpects a name\x1d")?;
                options.scopes = if value.eq_ignore_ascii_case("all") {
                    Scope::ALL.to_vec()
                } else {
                    vec![Scope::from_slug(value).with_context(|| {
                        format!("\x1eUnbekannter Scope\x1funknown scope\x1d: {value}")
                    })?]
                };
            }
            other => bail!(
                "\x1eUnbekannte Option\x1funknown option\x1d: {other}\n\n{}",
                help()
            ),
        }
    }

    Ok(Command::Scan(ScanArgs {
        options,
        json,
        quiet,
    }))
}

/// The smallest window `--window` will hand out, in physical pixels.
///
/// The pair the viewport is built with, read here as pixels. Below it the
/// window would refuse to shrink anyway, and a picture of a window arguing
/// with its own minimum is worth nothing.
const MIN_WINDOW: (i32, i32) = (900, 600);

/// `1600x1000` into a window size.
///
/// Too small is raised rather than refused: the number is a wish about how a
/// picture should look, and a run that stops over it costs more than it saves.
/// Anything that is not two numbers is refused, because there is no sensible
/// guess at what `--window gross` meant.
fn parse_window_size(value: &str) -> Result<(i32, i32)> {
    let lowered = value.to_ascii_lowercase();
    let (width, height) = lowered.split_once('x').with_context(|| {
        format!("--window \x1eerwartet <breite>x<höhe>\x1fexpects <width>x<height>\x1d: {value}")
    })?;

    let number = |part: &str| -> Result<i32> {
        part.trim()
            .parse::<i32>()
            .with_context(|| format!("\x1eKeine Zahl\x1fnot a number\x1d: {value}"))
    };

    Ok((
        number(width)?.max(MIN_WINDOW.0),
        number(height)?.max(MIN_WINDOW.1),
    ))
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
    // Says how far the scan reached. Without it `--all-types` and
    // `--every-type` differ only in how long they take, which is no way to
    // check that the second one did anything.
    if !args.options.file_types.is_empty() {
        crate::outln!(
            "\x1e{walked} Dateitypen untersucht, {found} davon registriert\
              \x1f{walked} file types examined, {found} of them registered\x1d",
            walked = args.options.file_types.len(),
            found = result.file_types.len()
        );
    }
    crate::outln!();

    crate::outln!(
        "{:<7} {:<8} {:<22} {:<34} {:<7} Befehl|CLSID",
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
    {
        use crate::program::identity::Presence;
        let count = |wanted: Presence| groups.iter().filter(|g| g.presence == wanted).count();
        crate::outln!(
            "\x1eProgramme vorhanden\x1fpresent\x1d: {}, \x1enicht mehr da\x1fgone\x1d: {}, \x1enicht prüfbar\x1funknown\x1d: {}",
            count(Presence::Present),
            count(Presence::Missing),
            count(Presence::Unknown)
        );
    }
    crate::outln!();

    for group in &groups {
        let marks = [
            group.is_system.then_some("System"),
            group.read_only.then_some("schreibgeschützt"),
            (!group.clsids.is_empty()).then_some("COM"),
            // The window paints this row red; the console has no colour, so it
            // says it in words.
            (group.presence == crate::program::identity::Presence::Missing)
                .then_some("\x1enicht mehr vorhanden\x1fgone\x1d"),
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
        .with_context(|| format!("\x1eKeine Erweiterung\x1fnot an extension\x1d: {raw_ext}"))?;

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
            "\x1eSchlüssel existiert nicht\x1fkey does not exist\x1d: {}",
            target.full_path()
        );
    }

    let plan = Plan::new(
        action.label(),
        vec![Operation {
            display_name: target.relative().to_string(),
            target: target.clone(),
            action,
            clsid: None,
        }],
    );

    let (direct, elevated) = plan.partition();
    let needs_elevation = !elevated.is_empty();

    if !confirmed {
        crate::outln!("\x1eWürde ausführen\x1fwould apply\x1d: {}", plan.label);
        crate::outln!("  {}", target.full_path());
        crate::outln!(
            "  \x1eRechteerhöhung nötig\x1felevation required\x1d: {}",
            if needs_elevation {
                "\x1eja\x1fyes\x1d"
            } else {
                "\x1enein\x1fno\x1d"
            }
        );
        crate::outln!("\x1eZum Ausführen --yes anhängen\x1fappend --yes to execute\x1d.");
        return Ok(());
    }

    let mut report = crate::registry::plan::execute(&direct)?;
    if needs_elevation {
        crate::outln!("\x1eStarte erhöhten Vorgang\x1fstarting elevated run …\x1d");
        report.merge(crate::elevation::run_elevated(&elevated)?);
    }

    // From the unelevated side, after the child is done.
    crate::elevation::notify_shell();

    crate::outln!(
        "{} \x1eerfolgreich\x1fsucceeded\x1d, {} \x1efehlgeschlagen\x1ffailed\x1d",
        report.succeeded(),
        report.failed()
    );
    // Both of them when the plan was split: the elevated half brings its own,
    // and that is the one the machine-wide changes hang on.
    for directory in &report.backup_directories {
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
            crate::errln!("\x1eFehler\x1ferror\x1d: {text}");
        } else {
            crate::errln!("\x1eWarnung\x1fwarning\x1d: {text}");
        }
    }

    let made = create::create(entry)?;
    // Without this the key is there and the running Explorer still shows the
    // old menu -- which looks exactly like a failed write.
    crate::elevation::notify_shell();

    // Beside the success, not instead of it: the key is written, and what
    // failed is the record in entries.json.
    if let Some(note) = &made.note {
        crate::errln!("\x1eWarnung\x1fwarning\x1d: {note}");
    }

    crate::outln!("\x1eAngelegt\x1fcreated\x1d: {}", made.target.full_path());
    if entry.is_submenu() {
        crate::outln!("  {} \u{25b8}", entry.display_name);
        for child in &entry.children {
            crate::outln!("    {} -> {}", child.display_name, child.command);
        }
    } else {
        crate::outln!("  {} -> {}", entry.display_name, entry.command);
    }
    crate::outln!("  entries.json: {}", create::entries_path()?.display());
    Ok(())
}

/// Lists what this tool created, as recorded in `entries.json`.
pub fn run_created() -> Result<()> {
    let recorded = crate::registry::create::recorded()?;
    if recorded.is_empty() {
        crate::outln!("\x1eNichts angelegt\x1fnothing created yet\x1d");
        return Ok(());
    }

    for entry in &recorded {
        let location = entry
            .target()
            .map(|t| t.full_path())
            .unwrap_or_else(|_| "?".into());
        crate::outln!("{:<28} {}", entry.display_name, location);
        if entry.is_submenu() {
            for child in &entry.children {
                crate::outln!("    \u{21b3} {:<22} {}", child.display_name, child.command);
            }
        } else {
            crate::outln!("    {}", entry.command);
        }
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
                bail!(
                    "\x1erun erwartet eine Kennung und eine Datei\x1fexpects an id and a file\x1d"
                );
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
                        format!("\x1eUnbekannte Kategorie\x1funknown category\x1d: {value}")
                    })?,
                    "--ext" => Category::ExtAssoc(value.clone()),
                    "--perceived" => Category::PerceivedType(value.clone()),
                    other => bail!("\x1eUnbekannte Option\x1funknown option\x1d: {other}"),
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
                    "--mode" => {
                        mode = value.to_lowercase();
                        if !matches!(mode.as_str(), "clipboard" | "open") {
                            bail!(
                                "\x1eUnbekannter Modus\x1funknown mode\x1d: {value} (clipboard, open)"
                            );
                        }
                    }
                    "--endpoint" => endpoint = Some(value.clone()),
                    "--field" => field = value.clone(),
                    "--icon" => icon = Some(value.clone()),
                    "--result" => {
                        result = value.to_lowercase();
                        if !matches!(result.as_str(), "save" | "open" | "report") {
                            bail!(
                                "\x1eUnbekanntes Ergebnis\x1funknown result\x1d: {value} (save, open, report)"
                            );
                        }
                    }
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
                    other => bail!("\x1eUnbekannte Option\x1funknown option\x1d: {other}"),
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
                    mode: WebMode::Upload(Box::new(Upload {
                        endpoint,
                        method: "POST".into(),
                        body: if raw {
                            UploadBody::Raw
                        } else {
                            UploadBody::Multipart { field }
                        },
                        headers,
                        fields: Vec::new(),
                        poll: None,
                        result: match result.as_str() {
                            "save" => ResultAction::Save { source, suffix },
                            "open" => ResultAction::Open { source },
                            _ => ResultAction::Report,
                        },
                    })),
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

        other => bail!("\x1eUnbekannt\x1funknown\x1d: favourite {other}"),
    }
}

pub fn run_favourite(command: FavouriteCommand) -> Result<()> {
    use crate::favourites;

    match command {
        FavouriteCommand::List => {
            let list = favourites::load()?;
            if list.is_empty() {
                crate::outln!("\x1eKeine Favoriten\x1fno favourites\x1d");
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
                // The console has no language setting, so it gets both halves.
                crate::errln!("\x1eHinweis\x1fnote\x1d: {}", problem.marked());
            }
            let id = favourites::add(*favourite)?;
            crate::outln!("\x1eAngelegt\x1fcreated\x1d: {id}");
            Ok(())
        }

        FavouriteCommand::Remove(id) => {
            favourites::remove(&id)?;
            crate::outln!("\x1eEntfernt\x1fremoved\x1d: {id}");
            Ok(())
        }

        FavouriteCommand::Place { id, category } => {
            let favourite = favourites::find(&id)?;
            let exe = std::env::current_exe().context("Eigenen Pfad nicht ermittelbar")?;
            let entry = favourite.entry(category, &exe);

            for problem in crate::registry::create::check(&entry) {
                crate::errln!("\x1eHinweis\x1fnote\x1d: {}", problem.message());
            }

            let made = crate::registry::create::create(&entry)?;
            crate::elevation::notify_shell();
            if let Some(note) = &made.note {
                crate::errln!("\x1eWarnung\x1fwarning\x1d: {note}");
            }
            crate::outln!("\x1eAngelegt\x1fcreated\x1d: {}", made.target.full_path());
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

/// Backs up every place this tool touches — the same set the window's
/// "back up everything" button takes.
pub fn run_backup_all() -> Result<()> {
    let paths = crate::registry::paths::full_backup_paths();
    let started = std::time::Instant::now();
    // The wide kind, like the button in the window: a branch this Windows
    // never had is noted, not removed again on a later restore.
    let token = backup::export_wide("gesamt", &paths)?;
    let elapsed = started.elapsed();

    let directory = token.directory();
    let manifest = backup::read_manifest(directory)?;
    // Size as well as count: "did it really take everything" is answered by
    // the megabytes, not by the number of files.
    let bytes: u64 = std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0);

    crate::outln!(
        "\x1e{saved} von {all} Schlüsseln gesichert in {seconds:.2} s\
          \x1f{saved} of {all} keys backed up in {seconds:.2} s\x1d, {megabytes:.1} MB",
        saved = manifest.entries.len(),
        all = paths.len(),
        seconds = elapsed.as_secs_f32(),
        megabytes = bytes as f64 / (1024.0 * 1024.0),
    );
    crate::outln!("{}", directory.display());
    if !manifest.missing.is_empty() {
        // Not a failure: not every category exists in every scope, and a
        // machine without a 32-bit classes tree is normal.
        crate::outln!(
            "\x1eNicht vorhanden\x1fnot present\x1d: {}",
            manifest.missing.len()
        );
    }
    Ok(())
}

pub fn run_backups() -> Result<()> {
    let backups = backup::list()?;
    if backups.is_empty() {
        // `display()`, not `{:?}`: the debug form doubles every backslash, so
        // the path it prints is one nobody can paste anywhere.
        crate::outln!(
            "\x1eKeine Backups unter\x1fno backups yet in\x1d {}",
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
    let report = backup::restore(path)?;
    crate::outln!(
        "{} Datei(en) \x1ezurückgespielt\x1frestored from\x1d {directory}",
        report.restored
    );
    if report.removed > 0 {
        crate::outln!(
            "{} \x1eSchlüssel entfernt, die es bei der Sicherung noch nicht ga\
             b\x1fkeys removed that did not exist when the backup was taken\x1d",
            report.removed
        );
    }
    // Every one of them, and after the count rather than instead of it: a
    // restore that stopped at the first gap used to leave the keys behind it
    // unreachable by this route.
    for failure in &report.failures {
        crate::outln!("  FEHL {failure}");
    }
    crate::outln!(
        "\x1eHinweis: reg import fügt hinzu und überschreibt, entfernt aber nichts.\
         \x1fnote: reg import adds and overwrites, it never removes.\x1d"
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
            "\x1eSchlüssel existiert nicht\x1fkey does not exist\x1d: {}",
            target.full_path()
        );
    }

    if !confirmed {
        crate::outln!("\x1eWürde sichern und löschen\x1fwould back up and delete\x1d:");
        crate::outln!("  {}", target.full_path());
        crate::outln!("\x1eZum Ausführen --yes anhängen\x1fappend --yes to execute\x1d.");
        return Ok(());
    }

    let token = backup::export_targets("delete", std::slice::from_ref(&target))?;
    crate::outln!("Backup: {}", token.directory().display());

    write::delete_tree(&target, &token)?;
    // The key is gone, so the record of having created it has to go too. The
    // plan path has done this all along (`plan.rs`); this one had not, so a
    // key deleted from the command line stayed listed in `entries.json` — and
    // that file is what the Windows 11 handler of ToDo 14 is meant to read.
    // Best effort, like there: the deletion succeeded, and failing to tidy the
    // bookkeeping is not a failed deletion.
    let _ = crate::registry::create::forget_target(&target);
    crate::outln!("\x1eGelöscht\x1fdeleted\x1d: {}", target.full_path());
    crate::outln!(
        "\x1eZurückholen mit\x1frestore with\x1d: ctxmenu restore \"{}\"",
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

    /// The `Start` behind a `--`something run, or a failed test.
    fn start_of(args: &[&str]) -> crate::app::Start {
        match parse_args(args).unwrap() {
            Command::Gui(start) => start,
            _ => panic!("{args:?} should open the window"),
        }
    }

    #[test]
    fn no_arguments_opens_the_window() {
        // This is a GUI application that happens to have a command line, not
        // the other way round: a bare double-click must show the product.
        let start = start_of(&[]);
        assert!(start.synthetic.is_none());
        assert!(start.bench.is_none());
        // Nothing imposed: the saved language stands and the window opens at
        // the size it always has.
        assert_eq!(start.language, None);
        assert_eq!(start.size, None);
        assert!(matches!(parse_args(&["--help"]).unwrap(), Command::Help));
    }

    #[test]
    fn the_start_language_can_be_named_for_one_run() {
        assert_eq!(
            start_of(&["--lang", "en"]).language,
            Some(crate::settings::Language::English)
        );
        assert_eq!(
            start_of(&["--lang", "Deutsch"]).language,
            Some(crate::settings::Language::German)
        );
        // It travels with the rest instead of ruling out the other flags: a
        // screenshot run names a language and a tab in the same breath.
        let start = start_of(&["--lang", "en", "--tab", "services"]);
        assert_eq!(start.language, Some(crate::settings::Language::English));
        assert_eq!(start.tab, crate::app::Tab::Services);

        assert!(parse_args(&["--lang", "klingonisch"]).is_err());
        assert!(parse_args(&["--lang"]).is_err());
    }

    #[test]
    fn the_window_size_is_parsed_and_never_falls_below_the_minimum() {
        assert_eq!(
            start_of(&["--window", "1600x1000"]).size,
            Some((1600, 1000))
        );
        // Upper case and spaces are what a person types, not a mistake.
        assert_eq!(
            start_of(&["--window", "1600X1000"]).size,
            Some((1600, 1000))
        );
        assert_eq!(
            start_of(&["--window", " 1600 x 1000 "]).size,
            Some((1600, 1000))
        );
        // Raised rather than refused, and raised per side: a window narrower
        // than its own minimum cannot be opened, so asking is not an error.
        assert_eq!(start_of(&["--window", "100x100"]).size, Some((900, 600)));
        assert_eq!(start_of(&["--window", "1600x10"]).size, Some((1600, 600)));

        for nonsense in ["abc", "1600", "1600x", "x1000", "1600x1000x60"] {
            assert!(
                parse_args(&["--window", nonsense]).is_err(),
                "--window {nonsense} should be refused"
            );
        }
        assert!(parse_args(&["--window"]).is_err());
    }

    #[test]
    fn both_help_texts_name_every_switch() {
        // The reason for this test: `services` was a tab `--tab` accepted and
        // the help did not mention, in either language, for as long as the tab
        // has existed. A list nobody compares drifts.
        for token in [
            "--tab",
            "--search",
            "--ext",
            "--lang",
            "--window",
            "--synthetic",
            "--bench",
            "--theme-probe",
            "--smoke",
            "--version",
            "--help",
            "categories",
            "filetypes",
            "programs",
            "favourites",
            "services",
            "backups",
        ] {
            assert!(HELP_DE.contains(token), "the German help omits {token}");
            assert!(HELP_EN.contains(token), "the English help omits {token}");
        }
    }

    #[test]
    fn the_version_is_asked_for_the_way_every_tool_answers_it() {
        for flag in ["--version", "-V", "version"] {
            assert!(
                matches!(parse_args(&[flag]).unwrap(), Command::Version),
                "{flag} must report the version"
            );
        }

        // One source, and it has to look like a version: the window title, the
        // command line and the file properties of the .exe all derive from
        // this string, so a malformed one would be wrong in three places.
        let parts: Vec<&str> = crate::VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "semantic versioning has three parts");
        assert!(
            parts.iter().all(|p| p.parse::<u32>().is_ok()),
            "every part is a number: {}",
            crate::VERSION
        );
    }

    #[test]
    fn the_synthetic_row_count_is_parsed() {
        let start = start_of(&["--synthetic", "2000"]);
        assert_eq!(start.synthetic, Some(2000));
        assert_eq!(start.bench, None);

        let start = start_of(&["--synthetic", "2000", "--bench", "600"]);
        assert_eq!(start.synthetic, Some(2000));
        assert_eq!(start.bench, Some(600));

        assert!(parse_args(&["--synthetic"]).is_err());
        assert!(parse_args(&["--synthetic", "viele"]).is_err());
    }

    #[test]
    fn a_bench_of_zero_frames_is_refused() {
        // Regression for todo 22: a run over zero frames never measures
        // anything, and used to hang the window forever instead of saying
        // so -- `bench.remaining -= 1` had nothing to reach zero from.
        let Err(error) = parse_args(&["--synthetic", "50", "--bench", "0"]) else {
            panic!("--bench 0 must be refused");
        };
        assert!(format!("{error}").contains("--bench 0"));

        // Zero synthetic rows is a perfectly fine (if boring) scan, and
        // unrelated to the counter this guards -- only --bench is refused.
        assert_eq!(start_of(&["--synthetic", "0"]).synthetic, Some(0));
    }

    fn favourite_args(args: &[&str]) -> Result<FavouriteCommand> {
        parse_favourite(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn a_typo_in_favourite_mode_is_refused_not_swallowed() {
        // Regression for todo 23: any unrecognised --mode silently became
        // WebMode::Clipboard, so "oepn" opened nothing and copied the URL to
        // the clipboard instead, without a word about the typo.
        let Err(error) =
            favourite_args(&["add", "--name", "X", "--url", "https://y", "--mode", "oepn"])
        else {
            panic!("a typo'd mode must be refused");
        };
        assert!(format!("{error}").contains("oepn"));

        // The real values still work, and build the mode that was named.
        let FavouriteCommand::Add(favourite) =
            favourite_args(&["add", "--name", "X", "--url", "https://y", "--mode", "open"])
                .expect("a known mode is accepted")
        else {
            panic!("expected FavouriteCommand::Add");
        };
        let crate::favourites::Tool::Web(web) = favourite.tool else {
            panic!("expected a web tool");
        };
        assert!(matches!(web.mode, crate::favourites::WebMode::Open { .. }));
    }

    #[test]
    fn a_typo_in_favourite_result_is_refused_not_swallowed() {
        // Same bug, the other switch: an unrecognised --result silently
        // became ResultAction::Report.
        let Err(error) = favourite_args(&[
            "add",
            "--name",
            "X",
            "--endpoint",
            "https://y",
            "--result",
            "svae",
        ]) else {
            panic!("a typo'd result must be refused");
        };
        assert!(format!("{error}").contains("svae"));

        let FavouriteCommand::Add(favourite) = favourite_args(&[
            "add",
            "--name",
            "X",
            "--endpoint",
            "https://y",
            "--result",
            "save",
        ])
        .expect("a known result is accepted") else {
            panic!("expected FavouriteCommand::Add");
        };
        let crate::favourites::Tool::Web(web) = favourite.tool else {
            panic!("expected a web tool");
        };
        let crate::favourites::WebMode::Upload(upload) = web.mode else {
            panic!("expected an upload mode");
        };
        assert!(matches!(
            upload.result,
            crate::favourites::ResultAction::Save { .. }
        ));
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
    fn a_submenu_is_assembled_from_repeated_sub_flags() {
        let Command::Create(entry) = parse_args(&[
            "create",
            "--category",
            "directory",
            "--name",
            "Werkzeuge",
            "--sub",
            "Zebra|cmd /c dir | more",
            "--sub-icon",
            r"shell32.dll,-244",
            "--sub",
            "Anton|cmd /c echo a",
        ])
        .unwrap() else {
            panic!("expected a create command");
        };

        assert!(entry.is_submenu());
        assert_eq!(entry.children.len(), 2);
        // Split at the *first* bar, so a pipeline survives in the command.
        assert_eq!(entry.children[0].display_name, "Zebra");
        assert_eq!(entry.children[0].command, "cmd /c dir | more");
        // The icon belongs to the `--sub` in front of it, not to the entry.
        assert_eq!(entry.children[0].icon.as_deref(), Some("shell32.dll,-244"));
        assert!(entry.children[1].icon.is_none());

        // Numbered in the order the flags came in, because that number is the
        // only thing that decides the order in the menu.
        assert_eq!(entry.children[0].key_name, "01_Zebra");
        assert_eq!(entry.children[1].key_name, "02_Anton");
    }

    #[test]
    fn a_malformed_sub_flag_is_refused_rather_than_guessed_at() {
        // No bar at all: guessing which half is the command would produce an
        // entry that looks right and does nothing.
        assert!(parse_args(&["create", "--name", "x", "--sub", "ohne Strich"]).is_err());
        // An icon with no entry to belong to.
        assert!(parse_args(&["create", "--name", "x", "--sub-icon", "a.dll,0"]).is_err());
    }

    #[test]
    fn truncation_keeps_character_boundaries() {
        assert_eq!(truncate("kurz", 10), "kurz");
        assert_eq!(truncate("äöüäöüäöü", 4), "äöü…");
        assert_eq!(truncate("abcdef", 6), "abcdef");
    }
}
