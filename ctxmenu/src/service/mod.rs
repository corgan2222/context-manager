//! Services described by an OpenAPI document, and the tools they offer.
//!
//! A favourite is one tool. This is the shelf they come from: an address, a
//! key, and whatever the service says about itself. Kept because the same
//! service is wanted again — a new tool appears in its description, the key is
//! rotated, a second machine gets the same set — and none of that should mean
//! filling in six fields by hand per tool.

pub mod grouping;
pub mod spec;

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// One service this program knows about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Service {
    /// Stable across renames, like a favourite's: it is what the tools made
    /// from this service point back to.
    pub id: String,
    pub name: String,
    /// Where the description lives, exactly as typed.
    pub spec_url: String,
    /// The header every request carries. Kept whole rather than as a bare key,
    /// because services disagree on the word in front of it — `Bearer`, `ApiKey`
    /// or nothing at all — and the whole line is what the user reads in the
    /// service's own documentation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<crate::favourites::Header>,
    /// Whether a plain `http://` address is acceptable. Off by default, and a
    /// service on the local network will need it on.
    #[serde(default)]
    pub allow_insecure: bool,
    /// Where the result address is named in an answer, as a dotted path.
    ///
    /// One setting per service rather than per tool: a service answers the
    /// same shape everywhere, and asking once beats asking a hundred times.
    #[serde(default)]
    pub result_path: String,
}

/// `%LOCALAPPDATA%\ctxmenu\services.json`
///
/// Beside `favourites.json` rather than inside it: a favourite is a tool that
/// works on its own, a service is where tools come from, and mixing the two
/// would make the file that already exists harder to read by hand.
pub fn path() -> Result<PathBuf> {
    let base =
        dirs::data_local_dir().context("\x1ekein LOCALAPPDATA\x1fno local data directory\x1d")?;
    Ok(base.join("ctxmenu").join("services.json"))
}

pub fn load() -> Result<Vec<Service>> {
    let path = path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("{}", path.display()))
}

/// Which entry of the list carries this id.
///
/// For `--service <id>`, where the id is typed by a human and a typo has to
/// say what would have worked instead: a list that answers "unknown" and
/// nothing else leaves the reader opening `services.json` in an editor to find
/// out what its own ids are. Case is ignored, the way every other identifier
/// on this command line is — `Tab::from_slug` and `Category::from_slug` both
/// lower-case first, and a service called `SnapOtter` is worth as little
/// typing as a tab called `Services`.
///
/// Kept here rather than in the window, because it looks at a list and at
/// nothing else: no file, no network, no state.
pub fn index_of(services: &[Service], id: &str) -> Result<usize> {
    let wanted = id.trim();
    if let Some(index) = services
        .iter()
        .position(|service| service.id.eq_ignore_ascii_case(wanted))
    {
        return Ok(index);
    }

    if services.is_empty() {
        anyhow::bail!(
            "\x1eKein Dienst mit der Kennung\x1fno service with the id\x1d {wanted}: \
             \x1ees ist überhaupt keiner eingerichtet\x1fthere is none set up at all\x1d"
        );
    }

    let known: Vec<&str> = services.iter().map(|service| service.id.as_str()).collect();
    anyhow::bail!(
        "\x1eKein Dienst mit der Kennung\x1fno service with the id\x1d {wanted}. \
         \x1eVorhanden\x1favailable\x1d: {}",
        known.join(", ")
    )
}

/// Whether text already on disk still parses as a service list.
///
/// Pulled out of `save` so the corrupted-file guard has a test that never
/// touches a real `%LOCALAPPDATA%\ctxmenu\services.json` — this only looks at
/// bytes already in memory.
fn readable(existing: &str) -> bool {
    serde_json::from_str::<Vec<Service>>(existing).is_ok()
}

/// Writes the list, but refuses to write over a file that does not parse.
///
/// `favourites::add`/`update`/`remove`/`shift` never risk overwriting a
/// damaged file: each loads its own list fresh from disk first and gives up
/// if that read fails. This function is handed the caller's whole list
/// instead of one change to make, so nothing else stands between a damaged
/// file and this call quietly replacing it with whatever the caller happens
/// to hold in memory — which, after a failed load kept an empty list rather
/// than erroring out, could be far short of what the user built up before.
/// So the same check is made here: a file that exists but no longer parses
/// stops the write instead of losing whatever it held.
pub fn save(services: &[Service]) -> Result<()> {
    let path = path()?;
    if let Ok(existing) = std::fs::read_to_string(&path)
        && !readable(&existing)
    {
        anyhow::bail!(
            "\x1e{path} ist beschädigt und wird nicht überschrieben. Bitte die \
             Datei prüfen oder aus einer Sicherung wiederherstellen.\x1f{path} is \
             damaged and will not be overwritten. Please check the file or \
             restore it from a backup.\x1d",
            path = path.display()
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("{}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(services)?;
    std::fs::write(&path, text).with_context(|| format!("{}", path.display()))
}

/// A service this program already knows the awkward answers for.
///
/// Not a directory of services and not an endorsement: an address on somebody
/// else's network is useless here, and the key is theirs. What a template saves
/// is the two fields nobody can guess from the outside -- where the answer names
/// the finished file, and whether plain `http://` has to be allowed -- plus a
/// hint of what the address looks like. Everything else the description says for
/// itself.
pub struct Template {
    pub name: &'static str,
    /// What the address usually looks like, as a hint in the empty field.
    pub address_hint: &'static str,
    pub result_path: &'static str,
    pub allow_insecure: bool,
}

/// The templates on offer. Deliberately short: a wrong entry here costs more
/// than a missing one, because it looks like knowledge.
pub const TEMPLATES: &[Template] = &[
    Template {
        // Self-hosted, so the address is always a private one and the key is
        // generated per installation. Measured against it on 2026-08-15.
        name: "SnapOtter",
        address_hint: "http://<host>:1349/api/docs/",
        result_path: "downloadUrl",
        allow_insecure: true,
    },
    Template {
        // The empty template: everything blank, for a service nobody has
        // written down yet. It exists so the picker is never a dead end.
        name: "",
        address_hint: "https://<host>/api/docs/",
        result_path: "",
        allow_insecure: false,
    },
];

/// Addresses worth trying for a description, best guess first.
///
/// What a user has in the clipboard is the page they were just reading —
/// `http://host:1349/api/docs/#tag/tools`, which is documentation for people and
/// not the document this program can read. Asking them to find the machine
/// readable one would mean explaining what `openapi.json` is; guessing costs one
/// failed request each and covers every generator seen so far.
pub fn spec_candidates(url: &str) -> Vec<String> {
    let url = url.trim().split('#').next().unwrap_or("").trim_end();
    if url.is_empty() {
        return Vec::new();
    }

    let mut out = vec![url.to_string()];
    // Already a document: nothing to guess.
    let tail = url.rsplit('/').next().unwrap_or("");
    if tail.ends_with(".json") || tail.ends_with(".yaml") || tail.ends_with(".yml") {
        return out;
    }

    let base = url.trim_end_matches('/');
    for name in ["openapi.json", "swagger.json", "openapi.yaml"] {
        out.push(format!("{base}/{name}"));
    }

    // And from the root, for services that document under one path and publish
    // under another.
    if let Some((scheme, rest)) = url.split_once("://") {
        let host = rest.split('/').next().unwrap_or(rest);
        let origin = format!("{scheme}://{host}");
        for path in [
            "/openapi.json",
            "/swagger.json",
            "/api/openapi.json",
            "/api-docs",
            "/v3/api-docs",
        ] {
            let candidate = format!("{origin}{path}");
            if !out.contains(&candidate) {
                out.push(candidate);
            }
        }
    }

    out
}

/// Fetches the description of a service and reads its tools.
///
/// Returns the address that actually answered along with the tools, so the
/// service can remember it and the next refresh is one request rather than six.
pub fn tools_of(service: &Service) -> Result<(String, Vec<spec::Tool>)> {
    let candidates = spec_candidates(&service.spec_url);
    if candidates.is_empty() {
        anyhow::bail!("\x1eKeine Adresse angegeben\x1fno address given\x1d");
    }
    if !service.allow_insecure
        && candidates
            .first()
            .is_some_and(|url| url.starts_with("http://"))
    {
        anyhow::bail!(
            "\x1eUnverschlüsseltes http:// ist für diesen Dienst nicht erlaubt\
             \x1funencrypted http:// is not allowed for this service\x1d"
        );
    }

    let headers: Vec<crate::favourites::Header> = service.auth_header.clone().into_iter().collect();

    let tried = candidates.len();
    let mut first_error = None;
    for candidate in candidates {
        let body = match crate::webtool::http::fetch(&candidate, &headers) {
            Ok(body) => body,
            Err(error) => {
                first_error.get_or_insert_with(|| format!("{error:#}"));
                continue;
            }
        };
        // HTML answers with 200 as happily as JSON does, so the parse is what
        // decides whether this address was the right one.
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body) else {
            continue;
        };
        let tools = spec::tools(&value);
        if !tools.is_empty() {
            return Ok((candidate, tools));
        }
        // A readable description without a single file endpoint is an answer,
        // not a miss: it means this service has nothing for a right-click.
        if value.get("paths").is_some() {
            return Ok((candidate, tools));
        }
    }

    // What the user needs is the next step, not the first error: they gave an
    // address that exists, and the answer is that the machine readable document
    // is somewhere else. Whether the first guess was a 404 or a timeout does
    // not change what to do about it, so it goes at the end.
    anyhow::bail!(
        "\x1eUnter dieser Adresse steht keine maschinenlesbare Beschreibung \
         (OpenAPI/Swagger). {tried} Adresse(n) geprüft. Wenn der Dienst eine \
         hat, ist sie meist als openapi.json verlinkt — diese Adresse direkt \
         eintragen.\x1fno machine readable description (OpenAPI/Swagger) at this \
         address; {tried} address(es) tried. If the service has one it is usually \
         linked as openapi.json — give that address directly.\x1d{}",
        match first_error {
            Some(error) => format!(" [{error}]"),
            None => String::new(),
        }
    )
}

/// A readable, stable id from a name — the same rule favourites use.
pub fn id_for(name: &str) -> String {
    // Cut by characters rather than by bytes. `String::truncate` insists on a
    // character boundary and panics otherwise, and the names this is handed
    // come out of somebody else's description: a summary of 47 letters followed
    // by an umlaut is an ordinary German sentence, not a corner case. In a
    // release build that panic takes the window with it, without a message.
    let id: String = name
        .to_lowercase()
        .chars()
        .map(|c| match c.is_alphanumeric() {
            true => c,
            false => '_',
        })
        .take(48)
        .collect();
    match id.trim_matches('_').is_empty() {
        true => "dienst".into(),
        false => id,
    }
}

/// The favourite one tool of this service becomes.
///
/// Everything a favourite needs is either in the service (address, key, where
/// the result is named) or in the tool (path, field, name) — which is the whole
/// reason for keeping services at all.
pub fn favourite_for(
    service: &Service,
    tool: &spec::Tool,
    settings: Option<String>,
    suffix: &str,
) -> crate::favourites::Favourite {
    use crate::favourites::{
        Favourite, ResultAction, ResultSource, Tool, Upload, UploadBody, WebMode, WebTool,
    };

    let name = format!("{}: {}", service.name, tool.summary);
    let fields = match (settings, &tool.settings) {
        // One field holds everything the service wants: what was typed goes in
        // under that name, as the JSON the service asked for.
        (Some(settings), spec::Settings::Text { field, .. })
        | (
            Some(settings),
            spec::Settings::Fields {
                field: Some(field), ..
            },
        ) => {
            vec![crate::favourites::Header {
                name: field.clone(),
                value: settings,
            }]
        }
        // Each option is a form field of its own, so the object the panel
        // filled in is spread back out into them.
        (Some(settings), spec::Settings::Fields { field: None, .. }) => spread(&settings),
        _ => Vec::new(),
    };

    Favourite {
        id: format!("{}__{}", service.id, id_for(&tool.summary)),
        name,
        icon: None,
        note: Some(tool.path.clone()),
        tool: Tool::Web(WebTool {
            mode: WebMode::Upload(Box::new(Upload {
                endpoint: endpoint_for(service, tool),
                method: tool.method.clone(),
                body: UploadBody::Multipart {
                    field: tool.file_field.clone(),
                },
                headers: service.auth_header.clone().into_iter().collect(),
                fields,
                // Only when the description says where to ask. Whether *this*
                // endpoint queues cannot be read out of a description — the
                // test service answers `200` and `202` to the same request by
                // turns — so every tool of a service that offers the path
                // carries it, and it is used on the clicks where it is needed.
                poll: match tool.progress.trim().is_empty() {
                    true => None,
                    false => Some(crate::favourites::Poll::at(tool.progress.trim())),
                },
                result: ResultAction::Save {
                    source: match service.result_path.trim().is_empty() {
                        true => ResultSource::Body,
                        false => ResultSource::Json {
                            path: service.result_path.clone(),
                        },
                    },
                    suffix: suffix.to_string(),
                },
            })),
            allow_insecure: service.allow_insecure,
            // The service was confirmed as a whole when it was added; asking
            // again per tool would be the same question a hundred times.
            confirmed: true,
        }),
    }
}

/// One form field per option, out of the object the panel filled in.
///
/// A service that declares every option as its own form field cannot be sent
/// one field holding JSON — it would read none of it. So `{"width":1920}`
/// travels as a part named `width` holding `1920`. A string keeps its text; a
/// number or a flag is written the way JSON writes it, which is the way the
/// service spelled it out in its own description.
fn spread(settings: &str) -> Vec<crate::favourites::Header> {
    let Ok(serde_json::Value::Object(values)) = serde_json::from_str(settings) else {
        return Vec::new();
    };
    values
        .into_iter()
        .map(|(name, value)| crate::favourites::Header {
            name,
            value: match value {
                serde_json::Value::String(text) => text,
                other => other.to_string(),
            },
        })
        .collect()
}

/// The full address of one tool: what the description says its paths hang
/// under, plus the tool's own path.
///
/// The `servers` block is the service's own word on where its interface lives,
/// and it comes in three shapes. An address of its own replaces the origin —
/// a description may be published behind a documentation server or a proxy and
/// describe a machine somewhere else. A path (`/api/v1`) hangs under the origin
/// the description was fetched from. And `/`, which the test service writes,
/// means the origin itself: joined naively it would grow the second slash that
/// makes every request fail.
fn endpoint_for(service: &Service, tool: &spec::Tool) -> String {
    let origin = service
        .spec_url
        .split_once("://")
        .map(|(scheme, rest)| {
            let host = rest.split('/').next().unwrap_or(rest);
            format!("{scheme}://{host}")
        })
        .unwrap_or_else(|| service.spec_url.clone());

    let base = tool.base.trim();
    let root = match base.contains("://") {
        true => base.to_string(),
        false => joined(&origin, base),
    };
    joined(&root, &tool.path)
}

/// Two halves of an address with exactly one slash between them, however many
/// each half brought along.
fn joined(left: &str, right: &str) -> String {
    format!(
        "{}/{}",
        left.trim_end_matches('/'),
        right.trim_start_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> Service {
        Service {
            id: "snapotter".into(),
            name: "SnapOtter".into(),
            spec_url: "http://192.168.2.11:1349/api/docs/openapi.json".into(),
            auth_header: Some(crate::favourites::Header {
                name: "Authorization".into(),
                value: "Bearer si_test".into(),
            }),
            allow_insecure: true,
            result_path: "downloadUrl".into(),
        }
    }

    fn tool() -> spec::Tool {
        spec::Tool {
            path: "/api/v1/tools/image/compress".into(),
            // What the test service writes: paths hang under the origin itself.
            base: "/".into(),
            progress: "/api/v1/jobs/{jobId}/progress".into(),
            method: "POST".into(),
            tag: Some("Tools".into()),
            summary: "Compress Image".into(),
            description: None,
            file_field: "file".into(),
            settings: spec::Settings::None,
            usable: spec::Usable::Yes,
        }
    }

    #[test]
    fn a_tool_becomes_a_favourite_that_carries_the_services_own_answers() {
        use crate::favourites::{ResultAction, ResultSource, Tool, UploadBody, WebMode};

        let favourite = favourite_for(&service(), &tool(), None, ".klein");

        assert_eq!(favourite.id, "snapotter__compress_image");
        assert_eq!(favourite.name, "SnapOtter: Compress Image");

        let Tool::Web(web) = &favourite.tool else {
            panic!("a service tool is a web tool");
        };
        assert!(web.allow_insecure, "taken from the service");
        let WebMode::Upload(upload) = &web.mode else {
            panic!("expected an upload");
        };

        // The address is the origin of the description plus the tool's path —
        // the description itself sits under /api/docs/ and must not leak in.
        assert_eq!(
            upload.endpoint,
            "http://192.168.2.11:1349/api/v1/tools/image/compress"
        );
        assert_eq!(upload.headers.len(), 1, "the service's key travels along");
        assert_eq!(
            upload.body,
            UploadBody::Multipart {
                field: "file".into()
            }
        );
        assert_eq!(
            upload.result,
            ResultAction::Save {
                source: ResultSource::Json {
                    path: "downloadUrl".into()
                },
                suffix: ".klein".into()
            }
        );

        // The way back to a queued job, taken from the description. Whether
        // this endpoint queues cannot be known here -- the same request is
        // answered `200` and `202` by turns -- so it is written down for every
        // tool and used on the clicks that need it.
        let poll = upload.poll.as_ref().expect("the description offers one");
        assert_eq!(poll.path, "/api/v1/jobs/{jobId}/progress");
        assert_eq!(poll.job, "jobId");
        assert!(
            poll.result.is_empty(),
            "the result stands where the ordinary answer names it"
        );
    }

    #[test]
    fn a_service_that_says_nothing_about_jobs_gets_no_way_back() {
        let mut tool = tool();
        tool.progress = String::new();

        let favourite = favourite_for(&service(), &tool, None, ".neu");
        let crate::favourites::Tool::Web(web) = &favourite.tool else {
            unreachable!()
        };
        let crate::favourites::WebMode::Upload(upload) = &web.mode else {
            unreachable!()
        };
        assert_eq!(
            upload.poll, None,
            "nothing to guess at, so nothing is written down"
        );
    }

    #[test]
    fn settings_travel_as_the_form_field_the_service_named() {
        let mut tool = tool();
        tool.settings = spec::Settings::Text {
            field: "settings".into(),
            description: None,
        };

        let favourite = favourite_for(&service(), &tool, Some(r#"{"width":1920}"#.into()), ".neu");
        let crate::favourites::Tool::Web(web) = &favourite.tool else {
            unreachable!()
        };
        let crate::favourites::WebMode::Upload(upload) = &web.mode else {
            unreachable!()
        };

        assert_eq!(upload.fields.len(), 1);
        assert_eq!(upload.fields[0].name, "settings");
        assert_eq!(upload.fields[0].value, r#"{"width":1920}"#);
    }

    #[test]
    fn a_service_without_a_result_path_expects_the_file_itself() {
        use crate::favourites::{ResultAction, ResultSource};

        let mut service = service();
        service.result_path = String::new();

        let favourite = favourite_for(&service, &tool(), None, ".neu");
        let crate::favourites::Tool::Web(web) = &favourite.tool else {
            unreachable!()
        };
        let crate::favourites::WebMode::Upload(upload) = &web.mode else {
            unreachable!()
        };

        assert_eq!(
            upload.result,
            ResultAction::Save {
                source: ResultSource::Body,
                suffix: ".neu".into()
            }
        );
    }

    #[test]
    fn the_page_a_user_copies_leads_to_the_document_this_program_needs() {
        let list = spec_candidates("http://192.168.2.11:1349/api/docs/#tag/tools");

        // The fragment is gone — it addresses a place on a page, not a resource.
        assert_eq!(list[0], "http://192.168.2.11:1349/api/docs/");
        assert!(list.contains(&"http://192.168.2.11:1349/api/docs/openapi.json".into()));
        assert!(list.contains(&"http://192.168.2.11:1349/openapi.json".into()));
        assert!(list.contains(&"http://192.168.2.11:1349/v3/api-docs".into()));
    }

    #[test]
    fn an_address_that_already_names_a_document_is_not_guessed_at() {
        let list = spec_candidates("https://transmute.sh/openapi.json");
        assert_eq!(list, vec!["https://transmute.sh/openapi.json"]);
    }

    #[test]
    fn nothing_is_tried_for_an_empty_address() {
        assert!(spec_candidates("   ").is_empty());
        assert!(spec_candidates("#tag/tools").is_empty());
    }

    #[test]
    fn ids_stay_readable_and_usable_as_file_names() {
        assert_eq!(id_for("SnapOtter"), "snapotter");
        assert_eq!(id_for("Bild verkleinern!"), "bild_verkleinern_");
        assert_eq!(id_for("   "), "dienst");
    }

    #[test]
    fn a_long_name_is_cut_where_a_character_ends_not_where_a_byte_does() {
        // 47 letters and then an umlaut, so the 48th character is two bytes
        // wide and the cut falls in the middle of it. Cutting by bytes panics
        // here, and the name comes out of somebody else's description.
        let name = format!("{}ä{}", "a".repeat(47), "b".repeat(20));
        let id = id_for(&name);

        assert_eq!(id.chars().count(), 48);
        assert!(id.ends_with('ä'), "the whole character survives: {id}");
        // Every kind of writing, cut at the same place.
        assert_eq!(id_for(&"д".repeat(60)).chars().count(), 48);
        assert_eq!(id_for(&"漢".repeat(60)).chars().count(), 48);
        assert_eq!(id_for(&"a".repeat(60)).len(), 48);
    }

    #[test]
    fn the_address_follows_the_servers_entry_whichever_shape_it_has() {
        let service = service();

        // `/` is the origin itself -- and the one that used to grow a second
        // slash, which is what the acceptance test watches.
        let mut tool = tool();
        tool.base = "/".into();
        assert_eq!(
            endpoint_for(&service, &tool),
            "http://192.168.2.11:1349/api/v1/tools/image/compress"
        );

        // A description that says nothing is the same as `/`.
        tool.base = String::new();
        assert_eq!(
            endpoint_for(&service, &tool),
            "http://192.168.2.11:1349/api/v1/tools/image/compress"
        );

        // A path hangs under the origin the description came from.
        tool.base = "/gateway".into();
        assert_eq!(
            endpoint_for(&service, &tool),
            "http://192.168.2.11:1349/gateway/api/v1/tools/image/compress"
        );

        // An address of its own replaces it: the interface may live on another
        // machine than the document that describes it.
        tool.base = "https://api.example.com/v2/".into();
        tool.path = "tools/compress".into();
        assert_eq!(
            endpoint_for(&service, &tool),
            "https://api.example.com/v2/tools/compress"
        );
    }

    #[test]
    fn a_path_without_a_leading_slash_does_not_grow_into_the_host() {
        let mut tool = tool();
        tool.base = String::new();
        tool.path = "api/v1/tools/image/compress".into();

        assert_eq!(
            endpoint_for(&service(), &tool),
            "http://192.168.2.11:1349/api/v1/tools/image/compress"
        );
    }

    #[test]
    fn settings_declared_as_separate_form_fields_travel_under_their_own_names() {
        let mut tool = tool();
        tool.settings = spec::Settings::Fields {
            field: None,
            fields: Vec::new(),
        };

        let favourite = favourite_for(
            &service(),
            &tool,
            Some(r#"{"format":"png","quality":80,"strip":true}"#.into()),
            ".neu",
        );
        let crate::favourites::Tool::Web(web) = &favourite.tool else {
            unreachable!()
        };
        let crate::favourites::WebMode::Upload(upload) = &web.mode else {
            unreachable!()
        };

        let sent: Vec<(&str, &str)> = upload
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field.value.as_str()))
            .collect();
        // Three parts, each under the name the description gave it -- not one
        // part holding JSON, which such a service would read none of. A string
        // travels as its text, without the quotes JSON writes around it.
        assert_eq!(
            sent,
            vec![("format", "png"), ("quality", "80"), ("strip", "true")]
        );
    }

    #[test]
    fn readable_accepts_a_service_list_and_rejects_broken_json() {
        assert!(readable("[]"));
        assert!(readable(
            r#"[{"id":"snapotter","name":"SnapOtter","spec_url":"http://x/","allow_insecure":false,"result_path":""}]"#
        ));
        // A crash mid-write is the case `save` has to catch: neither empty
        // nor valid JSON, just whatever made it to disk before the write cut
        // off.
        assert!(!readable(""));
        assert!(!readable("{ this is not json"));
        // The right shape of JSON but the wrong type -- an object where the
        // list belongs -- is just as unreadable as a service list.
        assert!(!readable(r#"{"not":"a list"}"#));
    }

    #[test]
    fn a_service_is_found_by_its_id_whatever_the_case() {
        let mut second = service();
        second.id = "otherhouse".into();
        let services = vec![service(), second];

        assert_eq!(index_of(&services, "snapotter").unwrap(), 0);
        assert_eq!(index_of(&services, "SnapOtter").unwrap(), 0);
        // A trailing space survives a copied command line more often than
        // anyone would like.
        assert_eq!(index_of(&services, " otherhouse ").unwrap(), 1);
    }

    #[test]
    fn an_unknown_id_says_which_ones_there_are() {
        let services = vec![service()];
        let message = format!("{:#}", index_of(&services, "snapotters").unwrap_err());

        // The typo is echoed, and so is the id that would have worked --
        // without them the reader has to open services.json to find out.
        assert!(message.contains("snapotters"), "{message}");
        assert!(message.contains("snapotter"), "{message}");

        // Nothing set up at all is its own answer: a list of available ids
        // would be an empty one, which says nothing.
        let message = format!("{:#}", index_of(&[], "snapotter").unwrap_err());
        assert!(message.contains("snapotter"), "{message}");
        assert!(
            crate::bilingual::pick(&message, crate::settings::Language::English)
                .contains("none set up"),
            "{message}"
        );
    }
}
