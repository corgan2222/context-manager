//! Der Werkzeugkasten: Programme und Webtools, die man einmal einträgt.
//!
//! Favourites live *in this application*, not in the Explorer menu. They are a
//! palette: a tool goes in once and stays, and from there it can be written
//! into any category or file type as an ordinary context menu entry. Nothing
//! here creates a "Favourites" submenu — that was considered and rejected.
//!
//! Two kinds of tool:
//!
//! * a **program**, which is what the registry can express by itself: the
//!   entry's command line runs the executable with the clicked file.
//! * a **web tool**, which the registry cannot express at all. A URL cannot
//!   read a local file — no browser lets a page do that — so the file has to
//!   be *sent*. The entry therefore calls this application back
//!   (`ctxmenu --favourite <id> "%1"`), and [`crate::webtool`] does the work.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::Category;
use crate::registry::create::NewEntry;

/// One saved tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Favourite {
    /// Stable across renames and reorderings, because it ends up inside the
    /// command line of every context menu entry made from this favourite.
    /// Renaming a favourite must not break entries already written.
    pub id: String,
    pub name: String,
    /// `datei,index` in the usual Windows notation, or a path to an `.ico`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub tool: Tool,
    /// Free text; not shown in menus, only in this application.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Tool {
    /// An executable. `args` is a template; `%1` is the clicked file, which is
    /// what Windows itself substitutes, so a program favourite needs no help
    /// from us at run time.
    Program {
        path: PathBuf,
        #[serde(default)]
        args: String,
    },
    Web(WebTool),
}

/// A tool that lives on the web.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebTool {
    /// Flattened on purpose. The file is meant to be readable, and possibly
    /// editable, by a person: `"mode": "clipboard", "url": "…"` says what it
    /// is, where `"mode": { "mode": "clipboard", … }` says it twice.
    #[serde(flatten)]
    pub mode: WebMode,
    /// Whether a plain `http://` address is acceptable.
    ///
    /// Off by default and deliberately per tool: uploading a file over an
    /// unencrypted connection hands it to everything on the way, and that is a
    /// decision, not a default.
    #[serde(default)]
    pub allow_insecure: bool,
    /// Set once the user has agreed to this tool sending files away. Kept per
    /// favourite, not globally: agreeing to one service says nothing about the
    /// next.
    #[serde(default)]
    pub confirmed: bool,
}

/// What clicking the entry actually does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum WebMode {
    /// Build an address from a template and open it. Nothing is transferred.
    ///
    /// For tools that only need a name — a search, a wiki, a ticket form.
    Open { url: String },

    /// Put the file on the clipboard, then open the page.
    ///
    /// The way that works with tools that have no interface for machines:
    /// Squoosh, the TinyPNG page, remove.bg. The page is opened, the file is
    /// on the clipboard, one Ctrl+V finishes it. No key, no endpoint, and it
    /// works with tools that never planned for this.
    Clipboard { url: String },

    /// Send the file and do something with what comes back.
    Upload(Upload),
}

/// An HTTP request that carries the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Upload {
    pub endpoint: String,
    /// `POST` unless a service insists otherwise.
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(flatten)]
    pub body: UploadBody,
    #[serde(default)]
    pub headers: Vec<Header>,
    /// Additional plain form fields, for `multipart` only.
    #[serde(default)]
    pub fields: Vec<Header>,
    #[serde(flatten)]
    pub result: ResultAction,
}

fn default_method() -> String {
    "POST".into()
}

/// A header or form field. A pair, but named, because a tuple in JSON reads
/// like a riddle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "body", rename_all = "lowercase")]
pub enum UploadBody {
    /// `multipart/form-data` with the file in one field. What most upload
    /// forms and self-hosted tools expect.
    Multipart {
        #[serde(default = "default_field")]
        field: String,
    },
    /// The file, unwrapped, as the whole request body. What several APIs want
    /// — TinyPNG's `shrink` endpoint among them.
    Raw,
}

fn default_field() -> String {
    "file".into()
}

/// What to do with the answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum ResultAction {
    /// Write the result next to the original file.
    Save {
        #[serde(default)]
        source: ResultSource,
        /// Appended before the extension: `bild.png` plus `.min` becomes
        /// `bild.min.png`. An existing file is never overwritten; a counter is
        /// added instead.
        #[serde(default = "default_suffix")]
        suffix: String,
    },
    /// Open the address the service answered with.
    Open {
        #[serde(default)]
        source: ResultSource,
    },
    /// Only report what happened.
    Report,
}

fn default_suffix() -> String {
    ".neu".into()
}

/// Where the result is to be found in the answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "lowercase")]
pub enum ResultSource {
    /// The answer *is* the file.
    #[default]
    Body,
    /// The answer names it in the `Location` header.
    Location,
    /// The answer is JSON and names it in a field, addressed as `output.url`.
    Json { path: String },
}

impl Favourite {
    /// The command line for a context menu entry made from this favourite.
    ///
    /// `exe` is the path of *this* application, which only matters for web
    /// tools — a program favourite runs the program directly, with no detour
    /// through here.
    pub fn command_line(&self, exe: &Path) -> String {
        match &self.tool {
            Tool::Program { path, args } => {
                let args = args.trim();
                if args.is_empty() {
                    format!("\"{}\" \"%1\"", path.display())
                } else if args.contains("%1") || args.contains("%V") {
                    // The template already says where the file goes; putting a
                    // second one at the end would pass it twice.
                    format!("\"{}\" {args}", path.display())
                } else {
                    format!("\"{}\" {args} \"%1\"", path.display())
                }
            }
            Tool::Web(_) => format!("\"{}\" --favourite {} \"%1\"", exe.display(), self.id),
        }
    }

    /// The context menu entry this favourite becomes, in one category.
    ///
    /// Everything a favourite knows is already the answer to what an entry
    /// needs, which is the point of keeping the list: choosing where it should
    /// appear is the only decision left when adding it somewhere.
    pub fn entry(&self, category: Category, exe: &Path) -> NewEntry {
        NewEntry {
            category,
            // Derived from the id, not the name: the key name is the sort key
            // and cannot change under a rename without orphaning the old key.
            // The `ctxmenu_` prefix is what this program's own entries wear.
            key_name: format!("ctxmenu_{}", self.id),
            display_name: self.name.trim().to_string(),
            command: self.command_line(exe),
            icon: self.icon.clone().or_else(|| match &self.tool {
                // Without this the menu entry has no picture at all. The
                // program's own first icon is what Windows would show for the
                // program anyway, so it is the least surprising choice.
                Tool::Program { path, .. } => Some(format!("{},0", path.display())),
                Tool::Web(_) => None,
            }),
            position: None,
            extended: false,
            // A favourite is one tool and one command line, so it is one
            // entry. Grouping several favourites under a submenu would be a
            // different feature and a different decision.
            children: Vec::new(),
        }
    }

    /// The address this tool talks to, for anything that needs to show it.
    pub fn address(&self) -> Option<&str> {
        match &self.tool {
            Tool::Program { .. } => None,
            Tool::Web(web) => Some(match &web.mode {
                WebMode::Open { url } | WebMode::Clipboard { url } => url,
                WebMode::Upload(upload) => &upload.endpoint,
            }),
        }
    }

    /// Whether this tool sends the file anywhere.
    pub fn transfers_the_file(&self) -> bool {
        matches!(
            &self.tool,
            Tool::Web(WebTool {
                mode: WebMode::Upload(_),
                ..
            })
        )
    }

    /// Complaints about this favourite, empty when it is usable.
    ///
    /// Causes rather than sentences, for the same reason as
    /// [`crate::registry::create::check`]: this list feeds the window, which
    /// has a language setting, and the console, which has none.
    pub fn problems(&self) -> Vec<Fault> {
        let mut problems = Vec::new();

        if self.name.trim().is_empty() {
            problems.push(Fault::MissingName);
        }

        match &self.tool {
            Tool::Program { path, .. } => {
                if path.as_os_str().is_empty() {
                    problems.push(Fault::MissingPath);
                } else if !path.is_file() {
                    // A warning in effect: a program on a removable drive is
                    // legitimate, so this does not refuse anything.
                    problems.push(Fault::FileNotFound(path.display().to_string()));
                }
            }
            Tool::Web(web) => {
                let Some(url) = self.address() else {
                    return problems;
                };
                if url.trim().is_empty() {
                    problems.push(Fault::MissingAddress);
                } else if !url.starts_with("https://") {
                    if url.starts_with("http://") {
                        if !web.allow_insecure {
                            problems.push(Fault::InsecureAddress);
                        }
                    } else {
                        problems.push(Fault::NotHttps);
                    }
                }

                if let WebMode::Open { url } = &web.mode
                    && !url.contains('{')
                {
                    problems.push(Fault::NoPlaceholder);
                }
            }
        }

        problems
    }
}

/// What is wrong with a favourite, without saying it in any particular
/// language.
///
/// The counterpart to [`crate::registry::create::Fault`], and here for the
/// same reason: the window formulates in the language it is set to, the
/// console prints both halves because it has no setting to follow.
#[derive(Debug, Clone, PartialEq)]
pub enum Fault {
    MissingName,
    MissingPath,
    /// The program is not where the favourite says it is. Carries the path,
    /// because "file not found" without the name it looked for is no help.
    FileNotFound(String),
    MissingAddress,
    /// `http://` without the explicit permission that makes it acceptable.
    InsecureAddress,
    /// Neither `https://` nor `http://` — not an address at all.
    NotHttps,
    /// An "open address" tool whose URL never mentions the file.
    NoPlaceholder,
}

impl Fault {
    /// Both languages at once, for the console and the log.
    pub fn bilingual(&self) -> String {
        match self {
            Fault::MissingName => "Name fehlt / name is missing".into(),
            Fault::MissingPath => "Pfad fehlt / path is missing".into(),
            Fault::FileNotFound(path) => {
                format!("Datei nicht gefunden / file not found: {path}")
            }
            Fault::MissingAddress => "Adresse fehlt / address is missing".into(),
            Fault::InsecureAddress => {
                "Unverschlüsselte Adresse: die Datei ginge im Klartext durchs Netz. \
                 Nur mit ausdrücklicher Erlaubnis. / unencrypted address; the file \
                 would travel in the clear."
                    .into()
            }
            Fault::NotHttps => {
                "Adresse muss mit https:// beginnen / address must start with https://".into()
            }
            Fault::NoPlaceholder => {
                "Ohne Platzhalter wird die Datei gar nicht erwähnt — {name}, {stem}, \
                 {path} oder {ext} einsetzen. / without a placeholder the file is never \
                 mentioned."
                    .into()
            }
        }
    }
}

/// `%LOCALAPPDATA%\ctxmenu\favourites.json`
pub fn path() -> Result<PathBuf> {
    let base = dirs::data_local_dir().context("kein LOCALAPPDATA / no local data directory")?;
    Ok(base.join("ctxmenu").join("favourites.json"))
}

/// The saved list, in the order the user put it in.
///
/// A missing file is an empty list, not an error: nobody has added anything
/// yet. A *damaged* file is an error, because silently starting over would
/// throw away the list the user built.
pub fn load() -> Result<Vec<Favourite>> {
    let file = path()?;
    match std::fs::read_to_string(&file) {
        Ok(raw) if raw.trim().is_empty() => Ok(Vec::new()),
        Ok(raw) => serde_json::from_str(&raw).with_context(|| format!("{file:?}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(anyhow::Error::from(error).context(format!("{file:?}"))),
    }
}

/// Writes the list, via a temporary file.
///
/// Not pedantry: this file is read by every context menu entry that points at
/// a web tool. A half-written file caught by a click is a favourite that has
/// stopped existing.
pub fn save(list: &[Favourite]) -> Result<()> {
    let file = path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let temporary = file.with_extension("json.neu");
    std::fs::write(&temporary, serde_json::to_string_pretty(list)?)
        .with_context(|| format!("{temporary:?}"))?;
    std::fs::rename(&temporary, &file).with_context(|| format!("{file:?}"))?;
    Ok(())
}

/// Finds one, for the run-time path of a web tool.
pub fn find(id: &str) -> Result<Favourite> {
    load()?
        .into_iter()
        .find(|f| f.id == id)
        .with_context(|| format!("Kein Favorit mit dieser Kennung / no favourite with id {id}"))
}

/// Adds a favourite and hands back the id it got.
pub fn add(mut favourite: Favourite) -> Result<String> {
    let mut list = load()?;
    if favourite.id.trim().is_empty() {
        favourite.id = free_id(&favourite.name, &list);
    } else if list.iter().any(|f| f.id == favourite.id) {
        bail!(
            "Kennung schon vergeben / id already taken: {}",
            favourite.id
        );
    }

    let id = favourite.id.clone();
    list.push(favourite);
    save(&list)?;
    Ok(id)
}

/// Adds a whole batch in one write, replacing any that are already there.
///
/// One file write rather than one per tool: installing a category of a service
/// is fifty favourites at once, and fifty read-modify-write cycles would leave
/// half a tool box behind if one of them failed. Replacing rather than refusing
/// on a known id is what makes "install this category again after a refresh"
/// mean "bring these up to date" — the ids a service builds are stable.
///
/// Returns how many were new.
pub fn add_many(batch: Vec<Favourite>) -> Result<usize> {
    let mut list = load()?;
    let mut fresh = 0;

    for mut favourite in batch {
        if favourite.id.trim().is_empty() {
            favourite.id = free_id(&favourite.name, &list);
        }
        match list.iter_mut().find(|old| old.id == favourite.id) {
            Some(slot) => *slot = favourite,
            None => {
                fresh += 1;
                list.push(favourite);
            }
        }
    }

    save(&list)?;
    Ok(fresh)
}

/// Replaces one by id, keeping its position.
pub fn update(favourite: Favourite) -> Result<()> {
    let mut list = load()?;
    let slot = list
        .iter_mut()
        .find(|f| f.id == favourite.id)
        .with_context(|| {
            format!(
                "Kein Favorit mit dieser Kennung / no favourite with id {}",
                favourite.id
            )
        })?;
    *slot = favourite;
    save(&list)
}

pub fn remove(id: &str) -> Result<()> {
    let mut list = load()?;
    list.retain(|f| f.id != id);
    save(&list)
}

/// Moves one up or down; the order is the user's and is kept as given.
pub fn shift(id: &str, up: bool) -> Result<()> {
    let mut list = load()?;
    let Some(index) = list.iter().position(|f| f.id == id) else {
        return Ok(());
    };

    let other = if up {
        index.checked_sub(1)
    } else if index + 1 < list.len() {
        Some(index + 1)
    } else {
        None
    };

    if let Some(other) = other {
        list.swap(index, other);
        save(&list)?;
    }
    Ok(())
}

/// An id nobody in `list` holds.
///
/// Readable rather than random: it appears in the command line of every entry
/// made from this favourite, and somebody will eventually read it there while
/// wondering what a key does.
pub fn free_id(name: &str, list: &[Favourite]) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let stem = cleaned
        .trim_matches('_')
        .chars()
        .take(32)
        .collect::<String>();
    let stem = if stem.is_empty() {
        "werkzeug".to_string()
    } else {
        stem
    };

    let mut candidate = stem.clone();
    let mut counter = 2;
    while list.iter().any(|f| f.id == candidate) {
        candidate = format!("{stem}_{counter}");
        counter += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(name: &str, args: &str) -> Favourite {
        Favourite {
            id: "test".into(),
            name: name.into(),
            icon: None,
            note: None,
            tool: Tool::Program {
                path: PathBuf::from(r"C:\Windows\notepad.exe"),
                args: args.into(),
            },
        }
    }

    fn web(mode: WebMode) -> Favourite {
        Favourite {
            id: "web".into(),
            name: "Webtool".into(),
            icon: None,
            note: None,
            tool: Tool::Web(WebTool {
                mode,
                allow_insecure: false,
                confirmed: false,
            }),
        }
    }

    #[test]
    fn a_program_gets_the_clicked_file_exactly_once() {
        let exe = Path::new(r"C:\ctxmenu\ctxmenu.exe");

        // No template: the file is appended.
        assert_eq!(
            program("Editor", "").command_line(exe),
            r#""C:\Windows\notepad.exe" "%1""#
        );

        // A template that already places the file must not get a second one.
        assert_eq!(
            program("Editor", r#"--wait "%1""#).command_line(exe),
            r#""C:\Windows\notepad.exe" --wait "%1""#
        );

        // %V counts too: that is the background categories' placeholder.
        assert!(
            program("Editor", "-n %V")
                .command_line(exe)
                .matches("%1")
                .count()
                == 0
        );

        // Switches without a placeholder still get the file at the end.
        assert_eq!(
            program("Editor", "-n").command_line(exe),
            r#""C:\Windows\notepad.exe" -n "%1""#
        );
    }

    #[test]
    fn a_web_tool_routes_through_this_application() {
        let exe = Path::new(r"C:\ctxmenu\ctxmenu.exe");
        let mut favourite = web(WebMode::Clipboard {
            url: "https://squoosh.app".into(),
        });
        favourite.id = "png_verkleinern".into();

        // The id, not the name, is what stands in the command line — renaming
        // the favourite must not break entries already written.
        assert_eq!(
            favourite.command_line(exe),
            r#""C:\ctxmenu\ctxmenu.exe" --favourite png_verkleinern "%1""#
        );
    }

    #[test]
    fn an_unencrypted_address_is_refused_unless_it_was_chosen() {
        let mut favourite = web(WebMode::Upload(Upload {
            endpoint: "http://tool.local/shrink".into(),
            method: default_method(),
            body: UploadBody::Multipart {
                field: default_field(),
            },
            headers: Vec::new(),
            fields: Vec::new(),
            result: ResultAction::Report,
        }));

        assert!(
            favourite.problems().contains(&Fault::InsecureAddress),
            "http must be objected to: {:?}",
            favourite.problems()
        );

        // Explicitly allowed, it goes through — a tool in one's own network is
        // a legitimate case.
        if let Tool::Web(web) = &mut favourite.tool {
            web.allow_insecure = true;
        }
        assert!(
            favourite.problems().is_empty(),
            "{:?}",
            favourite.problems()
        );
    }

    #[test]
    fn an_address_without_a_placeholder_is_pointless_in_open_mode() {
        let favourite = web(WebMode::Open {
            url: "https://wiki.local/search".into(),
        });
        assert!(favourite.problems().contains(&Fault::NoPlaceholder));

        let favourite = web(WebMode::Open {
            url: "https://wiki.local/search?q={stem}".into(),
        });
        assert!(
            favourite.problems().is_empty(),
            "{:?}",
            favourite.problems()
        );
    }

    #[test]
    fn only_uploading_actually_sends_the_file() {
        assert!(!program("Editor", "").transfers_the_file());
        assert!(
            !web(WebMode::Clipboard {
                url: "https://x.example".into()
            })
            .transfers_the_file(),
            "the clipboard stays on this machine"
        );
        assert!(
            !web(WebMode::Open {
                url: "https://x.example/{stem}".into()
            })
            .transfers_the_file()
        );
        assert!(
            web(WebMode::Upload(Upload {
                endpoint: "https://x.example".into(),
                method: default_method(),
                body: UploadBody::Raw,
                headers: Vec::new(),
                fields: Vec::new(),
                result: ResultAction::Report,
            }))
            .transfers_the_file()
        );
    }

    #[test]
    fn an_entry_made_from_a_favourite_carries_everything_over() {
        let exe = Path::new(r"C:\ctxmenu\ctxmenu.exe");
        let mut favourite = program("Mit Editor öffnen", "-n");
        favourite.id = "editor".into();

        let entry = favourite.entry(Category::ExtAssoc(".txt".into()), exe);
        assert_eq!(entry.display_name, "Mit Editor öffnen");
        assert_eq!(entry.key_name, "ctxmenu_editor", "the id, not the name");
        assert_eq!(entry.command, r#""C:\Windows\notepad.exe" -n "%1""#);
        assert_eq!(
            entry.icon.as_deref(),
            Some(r"C:\Windows\notepad.exe,0"),
            "a menu entry with no icon looks broken next to the others"
        );

        // And it must land where that category lives.
        assert_eq!(
            entry.target().expect("creatable").relative(),
            r"SystemFileAssociations\.txt\shell\ctxmenu_editor"
        );
    }

    #[test]
    fn a_web_favourite_becomes_an_entry_that_calls_this_program() {
        let exe = Path::new(r"C:\ctxmenu\ctxmenu.exe");
        let mut favourite = web(WebMode::Clipboard {
            url: "https://squoosh.app".into(),
        });
        favourite.id = "squoosh".into();
        favourite.name = "PNG verkleinern".into();

        let entry = favourite.entry(Category::PerceivedType("image".into()), exe);
        assert!(entry.command.contains("--favourite squoosh"));
        assert!(
            entry.command.ends_with(r#""%1""#),
            "the file must be passed"
        );
        assert!(
            entry.icon.is_none(),
            "a web tool has no executable to take an icon from"
        );

        // No warnings from the entry checker either — a command it objects to
        // would be one the user has to overrule for no reason.
        assert!(
            crate::registry::create::check(&entry).is_empty(),
            "{:?}",
            crate::registry::create::check(&entry)
        );
    }

    #[test]
    fn ids_stay_readable_and_never_collide() {
        let list = [program("a", ""), program("b", "")];
        let taken: Vec<Favourite> = list
            .iter()
            .enumerate()
            .map(|(i, f)| Favourite {
                id: if i == 0 {
                    "png_verkleinern".into()
                } else {
                    "x".into()
                },
                ..f.clone()
            })
            .collect();

        assert_eq!(free_id("PNG verkleinern", &[]), "png_verkleinern");
        assert_eq!(free_id("PNG verkleinern", &taken), "png_verkleinern_2");
        assert_eq!(free_id("   ", &[]), "werkzeug");
        // Umlauts are not ASCII alphanumeric and become separators; what
        // matters is that the result is usable on a command line.
        assert!(!free_id("Bild größer", &[]).contains(' '));
    }

    #[test]
    fn the_saved_form_survives_a_round_trip() {
        let favourites = vec![
            program("Editor", "-n"),
            web(WebMode::Upload(Upload {
                endpoint: "https://api.tinify.com/shrink".into(),
                method: "POST".into(),
                body: UploadBody::Raw,
                headers: vec![Header {
                    name: "Authorization".into(),
                    value: "Basic …".into(),
                }],
                fields: Vec::new(),
                result: ResultAction::Save {
                    source: ResultSource::Json {
                        path: "output.url".into(),
                    },
                    suffix: ".min".into(),
                },
            })),
        ];

        let json = serde_json::to_string_pretty(&favourites).expect("serialises");
        let back: Vec<Favourite> = serde_json::from_str(&json).expect("reads back");
        assert_eq!(back, favourites);
    }

    #[test]
    fn defaults_fill_themselves_in_from_a_minimal_file() {
        // What a human would plausibly write by hand.
        let json = r#"[{
            "id": "verkleinern",
            "name": "PNG verkleinern",
            "tool": {
                "kind": "web",
                "mode": "upload",
                "endpoint": "https://tool.example/upload",
                "body": "multipart",
                "result": "report"
            }
        }]"#;

        let list: Vec<Favourite> = serde_json::from_str(json).expect("minimal form is enough");
        let Tool::Web(WebTool {
            mode: WebMode::Upload(upload),
            ..
        }) = &list[0].tool
        else {
            panic!("expected a web upload");
        };

        assert_eq!(upload.method, "POST");
        assert_eq!(
            upload.body,
            UploadBody::Multipart {
                field: "file".into()
            }
        );
        // A minimal hand-written entry still ends up as a real upload, which
        // is the case that has to ask before it sends anything.
        assert!(list[0].transfers_the_file());
    }
}
