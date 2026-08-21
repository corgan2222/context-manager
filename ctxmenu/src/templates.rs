//! The services this program already knows how to talk to.
//!
//! A web tool favourite takes seven fields nobody can guess: endpoint, method,
//! body shape, field name, header line, what to do with the answer and where
//! the answer names the file. For Imgur those seven read the same in every
//! piece of documentation on the internet, and typing them out is work this
//! program can do once for everybody.
//!
//! # Why a JSON file and not a `const` array
//!
//! [`service::TEMPLATES`](crate::service::TEMPLATES) is a `const` array
//! because it holds two fields per service. This holds a whole favourite per
//! service, times thirty-odd, and the list is the part most likely to grow by
//! somebody who has just used a service and knows what it wants. That person
//! should be able to add one by editing a text file, not by writing Rust.
//!
//! It is still compiled in: `include_str!` puts the file inside the `.exe`, so
//! the promise holds — one file, nothing to fetch, nothing to install. The
//! parse is checked by a test rather than at run time, which is why nothing
//! here returns a `Result`: a file that does not read cannot leave CI.
//!
//! # What a template is not
//!
//! It is not an endorsement and not a directory. Everything here was read off
//! the service's own documentation on 2026-08-21, and the same rule
//! `service::TEMPLATES` carries applies: **a wrong entry costs more than a
//! missing one, because it looks like knowledge.** A key is never in this
//! file. Where a service needs one, the header is there with the word in front
//! of it and nothing behind it, and the hint says where to get it.

use std::sync::LazyLock;

use serde::Deserialize;

use crate::favourites::{Favourite, Tool};
use crate::i18n::Strings;

/// The embedded catalogue.
const CATALOGUE: &str = include_str!("../templates.json");

/// Where the logos are served from.
///
/// One place rather than thirty-three, and the same address the README uses,
/// so a logo that moves moves once. The files themselves live in the site's
/// `public/icons`, which is checked in: pointing at each service's own server
/// would be a list that breaks quietly the day somebody redesigns their page.
const ICONS: &str = "https://corgan2222.github.io/context-manager/icons/";

/// The logos, in the binary.
///
/// The window has no image decoder and is not getting one
/// (`decisions/0027`): it draws icons through Windows, out of `.ico` files.
/// So the catalogue writes each logo it needs once into
/// `%LOCALAPPDATA%\ctxmenu\icons\`, wrapped by the same `ico_from_png` the
/// icon field uses, and hands the window a path.
///
/// Compiled in rather than fetched: a list of services that shows nothing
/// until the network answers is a list nobody trusts, and twenty-one small
/// PNGs are cheaper than that. The path in [`Template::icon`] stays a web
/// address, because that is what goes into the menu entry and what the README
/// links to.
const LOGOS: &[(&str, &[u8])] = &[
    // Not a template of this list: the two services that describe themselves
    // live in `service::TEMPLATES`, and their rows want a face just as much.
    (
        "snapotter",
        include_bytes!("../../website/public/icons/snapotter.png"),
    ),
    (
        "imgur",
        include_bytes!("../../website/public/icons/imgur.png"),
    ),
    (
        "imgbb",
        include_bytes!("../../website/public/icons/imgbb.png"),
    ),
    (
        "freeimage-host",
        include_bytes!("../../website/public/icons/freeimage-host.png"),
    ),
    (
        "catbox",
        include_bytes!("../../website/public/icons/catbox.png"),
    ),
    (
        "gofile",
        include_bytes!("../../website/public/icons/gofile.png"),
    ),
    (
        "removebg",
        include_bytes!("../../website/public/icons/removebg.png"),
    ),
    (
        "photoroom",
        include_bytes!("../../website/public/icons/photoroom.png"),
    ),
    (
        "stability-ai",
        include_bytes!("../../website/public/icons/stability-ai.png"),
    ),
    (
        "tinypng",
        include_bytes!("../../website/public/icons/tinypng.png"),
    ),
    (
        "stirling-pdf",
        include_bytes!("../../website/public/icons/stirling-pdf.png"),
    ),
    (
        "gotenberg",
        include_bytes!("../../website/public/icons/gotenberg.png"),
    ),
    (
        "nutrient",
        include_bytes!("../../website/public/icons/nutrient.png"),
    ),
    (
        "paste-rs",
        include_bytes!("../../website/public/icons/paste-rs.png"),
    ),
    (
        "bpast",
        include_bytes!("../../website/public/icons/bpast.png"),
    ),
    (
        "gitea",
        include_bytes!("../../website/public/icons/gitea.png"),
    ),
    (
        "forgejo",
        include_bytes!("../../website/public/icons/forgejo.png"),
    ),
    (
        "zipline",
        include_bytes!("../../website/public/icons/zipline.png"),
    ),
    (
        "nextcloud",
        include_bytes!("../../website/public/icons/nextcloud.png"),
    ),
    (
        "bunny",
        include_bytes!("../../website/public/icons/bunny.png"),
    ),
    (
        "minio",
        include_bytes!("../../website/public/icons/minio.png"),
    ),
    (
        "virustotal",
        include_bytes!("../../website/public/icons/virustotal.png"),
    ),
];

/// One ready-made favourite, plus what the window needs to place it.
#[derive(Debug, Clone, Deserialize)]
pub struct Template {
    /// Also the name of the logo file, and what the README links to.
    pub id: String,
    pub name: String,
    pub group: Group,
    /// The perceived type this entry usually belongs to -- `image`, `video`,
    /// `audio`, `text`, `compressed` -- or empty where no one kind fits, which
    /// is the case for everything that takes any file at all.
    #[serde(default)]
    pub category: String,
    /// The service's own page, for the reader rather than the program.
    pub home: String,
    /// What this does, in one half-sentence.
    ///
    /// The thing a person needs before anything else -- a list of names says
    /// nothing about what any of them is for. It ends up in the tooltip of the
    /// row, in the README and on the site, which is why it lives here and not
    /// in three places.
    pub what: Hint,
    pub hint: Hint,
    #[serde(flatten)]
    pub tool: Tool,
}

/// The one sentence a template cannot do without: where the key comes from.
///
/// Two fields rather than the marked form the rest of the program uses. The
/// markers are three control characters, which JSON can only carry escaped,
/// and this file is meant to be edited by hand by somebody adding a service.
#[derive(Debug, Clone, Deserialize)]
pub struct Hint {
    pub de: String,
    pub en: String,
}

impl Hint {
    pub fn shown(&self) -> &str {
        match crate::bilingual::language() {
            crate::settings::Language::German => &self.de,
            crate::settings::Language::English => &self.en,
        }
    }
}

/// Roughly what a service is for, and the order the picker shows.
///
/// A key rather than a word: the window is bilingual, and a group called
/// `"Bilder"` in the file would be a file that only speaks German. The label
/// comes from [`Strings`] like every other piece of text on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    Images,
    Files,
    Editing,
    Documents,
    Text,
    Development,
    Storage,
    Security,
}

impl Group {
    /// Every group, in the order the picker draws them.
    ///
    /// Sharing first, because that is what most people came for, and the one
    /// that needs a server of your own last.
    ///
    /// The group that checks a file comes last, and it exists only because
    /// the address does not have to come out of the answer. VirusTotal answers
    /// an upload with an opaque job id and documents no way to turn that into
    /// a page a person can read; what it does document is that a file's own
    /// SHA-256 identifies it. So the address is built from the digest of the
    /// file that was just sent -- see `webtool::SHA256` -- and nothing is
    /// guessed. MalwareBazaar and Hybrid Analysis have the same shape and are
    /// still missing, because neither says what its page address looks like.
    pub const ALL: [Group; 8] = [
        Group::Images,
        Group::Files,
        Group::Editing,
        Group::Documents,
        Group::Text,
        Group::Development,
        Group::Storage,
        Group::Security,
    ];

    pub fn label(self, tr: &'static Strings) -> &'static str {
        match self {
            Group::Images => tr.tpl_group_images,
            Group::Files => tr.tpl_group_files,
            Group::Editing => tr.tpl_group_editing,
            Group::Documents => tr.tpl_group_documents,
            Group::Text => tr.tpl_group_text,
            Group::Development => tr.tpl_group_development,
            Group::Storage => tr.tpl_group_storage,
            Group::Security => tr.tpl_group_security,
        }
    }
}

impl Template {
    /// The address of this template's logo.
    pub fn icon(&self) -> String {
        format!("{ICONS}{}.png", self.id)
    }

    /// A path to this logo on disk, written once and reused.
    ///
    /// `None` where the file cannot be written, which is a missing picture in
    /// a list and never a reason to stop: the row still carries its name.
    pub fn logo(&self) -> Option<String> {
        logo_named(&self.id)
    }

    /// The favourite a click on this template starts from.
    ///
    /// The id is left empty, and `favourites::add` fills it in from the name
    /// against the list that exists. It must *not* be the template's own: an
    /// id ends up inside the command line of every menu entry made from the
    /// favourite, so two favourites from one template would write two entries
    /// that call the same one, and renaming either would break the other. The
    /// template's id names its logo and its row in the README, nothing else.
    pub fn favourite(&self) -> Favourite {
        Favourite {
            id: String::new(),
            name: self.name.clone(),
            // The local copy, not the web address. Saving a favourite runs its
            // icon field through `icons::web::localise`, which downloads what
            // it finds there -- and the address in `icon()` only answers once
            // the site has been published, so a fresh template used to fail to
            // save at all. The bytes are in the binary already.
            icon: self.logo(),
            tool: self.tool.clone(),
            note: None,
            from: Some(self.id.clone()),
        }
    }
}

/// A logo by name, for anything that is not a template of this list.
///
/// The two service templates carry one too, and they sit in
/// `service::TEMPLATES` rather than here -- so the catalogue asks by name.
pub fn logo_named(name: &str) -> Option<String> {
    let bytes = LOGOS
        .iter()
        .find(|(id, _)| *id == name)
        .map(|(_, bytes)| *bytes)?;
    crate::icons::web::stored(name, bytes).ok()
}

/// The catalogue, read once.
///
/// An empty list if the file does not parse, which is a state the test below
/// exists to make impossible. Better an empty picker than a window that will
/// not open over a data file.
pub fn all() -> &'static [Template] {
    static PARSED: LazyLock<Vec<Template>> =
        LazyLock::new(|| serde_json::from_str(CATALOGUE).unwrap_or_default());
    &PARSED
}

/// The template a favourite was made from, if it still exists.
///
/// `None` for one built by hand, for one read out of a ShareX file, and for a
/// service that has since left the catalogue -- all three are the same thing
/// to a caller: there is nothing more to say about this favourite than what
/// its own fields say.
pub fn by_id(id: Option<&str>) -> Option<&'static Template> {
    let id = id?;
    all().iter().find(|template| template.id == id)
}

/// The templates of one group, in file order.
pub fn of(group: Group) -> impl Iterator<Item = &'static Template> {
    all().iter().filter(move |template| template.group == group)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::favourites::{ResultAction, ResultSource, WebMode};

    #[test]
    fn the_catalogue_reads() {
        let parsed: Result<Vec<Template>, _> = serde_json::from_str(CATALOGUE);
        let templates = parsed.expect("templates.json has to parse");
        assert!(!templates.is_empty(), "an empty catalogue is a lost file");
        assert_eq!(
            templates.len(),
            all().len(),
            "`all` must not quietly swallow a parse error"
        );
    }

    /// Every field a person could get wrong while adding a service, checked in
    /// one pass. The failure this prevents is a template that looks like
    /// knowledge and sends nonsense to a real service.
    #[test]
    fn every_template_is_complete_and_unique() {
        let mut seen: Vec<&str> = Vec::new();

        for template in all() {
            let id = template.id.as_str();
            assert!(
                !id.is_empty()
                    && id
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "an id is the name of a file and of a README row: {id}"
            );
            assert!(!seen.contains(&id), "two templates called {id}");
            seen.push(id);

            assert!(!template.name.trim().is_empty(), "{id} has no name");
            assert!(
                template.home.starts_with("https://"),
                "{id} needs a page a reader can open"
            );
            assert!(
                !template.hint.de.trim().is_empty() && !template.hint.en.trim().is_empty(),
                "{id} needs its hint in both languages"
            );
            assert!(
                !template.what.de.trim().is_empty() && !template.what.en.trim().is_empty(),
                "{id} needs to say what it does, in both languages"
            );
            assert!(
                matches!(
                    template.category.as_str(),
                    "" | "image" | "video" | "audio" | "text" | "compressed"
                ),
                "{id} names a perceived type Windows does not have: {}",
                template.category
            );

            let Tool::Web(web) = &template.tool else {
                panic!("{id} is not a web tool, and a program template makes no sense here");
            };
            match &web.mode {
                WebMode::Upload(upload) => {
                    assert!(
                        upload.endpoint.starts_with("https://")
                            || (upload.endpoint.starts_with("http://") && web.allow_insecure),
                        "{id} sends over plain http without saying so"
                    );
                    for header in &upload.headers {
                        assert!(
                            !header.name.trim().is_empty(),
                            "{id} has a header without a name"
                        );
                    }
                    // A key never travels in this file. A header that carries
                    // one may hold the word in front of it -- `Bearer `,
                    // `Client-ID `, `token ` -- and nothing else, so the user
                    // can see where their own goes. Headers that carry
                    // something else, `accept` for one, are not checked: a
                    // value there is a value, not a leak.
                    for header in &upload.headers {
                        let name = header.name.to_ascii_lowercase();
                        let carries_a_key = matches!(
                            name.as_str(),
                            "authorization"
                                | "x-api-key"
                                | "x-apikey"
                                | "apikey"
                                | "api-key"
                                | "accesskey"
                                | "auth-key"
                                | "x-auth-token"
                                | "token"
                        );
                        assert!(
                            !carries_a_key
                                || header.value.is_empty()
                                || header.value.ends_with(' '),
                            "{id} looks like it ships a key: {}: {}",
                            header.name,
                            header.value
                        );
                    }
                    if let ResultAction::Save { source, .. } | ResultAction::Open { source } =
                        &upload.result
                        && let ResultSource::Json { path } | ResultSource::Built { url: path } =
                            source
                    {
                        assert!(!path.trim().is_empty(), "{id} points at nothing");
                    }
                }
                WebMode::Open { url } | WebMode::Clipboard { url } => {
                    assert!(url.starts_with("https://"), "{id} opens {url}");
                }
            }
        }
    }

    /// The logo is half the point of a template list, and a missing file shows
    /// up as a blank square in a menu rather than as an error anybody reads.
    #[test]
    fn every_template_has_its_logo_in_the_repository() {
        let icons = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the crate sits in the workspace")
            .join("website")
            .join("public")
            .join("icons");

        for template in all() {
            let file = icons.join(format!("{}.png", template.id));
            assert!(
                file.exists(),
                "no logo for {}: expected {}",
                template.id,
                file.display()
            );
            let bytes = std::fs::read(&file).expect("readable");
            assert_eq!(
                &bytes[..4],
                b"\x89PNG",
                "{} is not a PNG, and the icon path only wraps PNG and ICO",
                template.id
            );
            // The wrapper in `icons::web` refuses anything larger, and a
            // template that cannot produce an icon is a template with a blank
            // square in the menu.
            let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
            let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
            assert!(
                (16..=256).contains(&width) && (16..=256).contains(&height),
                "{} is {width}x{height}; a menu icon takes up to 256",
                template.id
            );
        }
    }

    #[test]
    fn a_template_leaves_the_id_to_the_list_it_joins() {
        let template = all().first().expect("at least one template");
        let made = template.favourite();

        assert!(
            made.id.is_empty(),
            "an id from the template would collide with the second favourite made from it"
        );
        assert_eq!(made.name, template.name);
    }

    /// The failure this prevents: saving the favourite fetched the icon field,
    /// the address was not published yet, the save failed and the window said
    /// nothing. Reported 2026-08-21.
    ///
    /// This writes into `%LOCALAPPDATA%\ctxmenu\icons\`, which is the one
    /// place under that directory a test may touch: the files there are copies
    /// of bytes compiled into this binary, written on demand and rewritten
    /// whenever they are missing. Nothing a person put there lives in it.
    #[test]
    fn a_template_hands_over_a_logo_that_needs_no_network() {
        for template in all() {
            let icon = template
                .favourite()
                .icon
                .unwrap_or_else(|| panic!("{} has no icon at all", template.id));
            assert!(
                !icon.starts_with("http"),
                "{} would have its icon downloaded on save: {icon}",
                template.id
            );
            assert!(
                std::path::Path::new(&icon).exists(),
                "{} names an icon that is not there: {icon}",
                template.id
            );
        }

        assert!(
            template_address_is_still_the_web_one(),
            "the web address stays, because the README and the site link to it"
        );
    }

    fn template_address_is_still_the_web_one() -> bool {
        all()
            .first()
            .is_some_and(|template| template.icon().starts_with("https://"))
    }

    /// The two lists that have to agree: a template added to the JSON without
    /// an `include_bytes!` beside it draws no picture, and nothing else says
    /// so.
    #[test]
    fn every_template_carries_its_logo_in_the_binary() {
        for template in all() {
            let found = LOGOS.iter().find(|(id, _)| *id == template.id);
            let (_, bytes) = found.unwrap_or_else(|| {
                panic!(
                    "{} has no line in LOGOS; add one beside the entry in templates.json",
                    template.id
                )
            });
            assert!(
                bytes.starts_with(&[0x89, b'P', b'N', b'G']),
                "{} is compiled in but is not a PNG",
                template.id
            );
        }

        // Every entry of LOGOS is either a template of this list or one of the
        // two service templates, and nothing else may sit in there unused.
        for (id, _) in LOGOS {
            assert!(
                all().iter().any(|template| template.id == *id)
                    || matches!(*id, "snapotter" | "stirling-pdf"),
                "LOGOS holds a logo nothing asks for: {id}"
            );
        }
    }

    /// Which tab a favourite lands in hangs off this one field.
    #[test]
    fn a_favourite_from_the_catalogue_says_where_it_came_from() {
        let template = all().first().expect("at least one template");
        assert_eq!(
            template.favourite().from.as_deref(),
            Some(template.id.as_str()),
            "without this the services tab cannot show what was picked there"
        );
    }

    #[test]
    fn every_group_has_something_in_it() {
        for group in Group::ALL {
            assert!(
                of(group).next().is_some(),
                "an empty group is a heading over nothing: {group:?}"
            );
        }
    }
}
