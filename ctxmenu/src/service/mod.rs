//! Services described by an OpenAPI document, and the tools they offer.
//!
//! A favourite is one tool. This is the shelf they come from: an address, a
//! key, and whatever the service says about itself. Kept because the same
//! service is wanted again — a new tool appears in its description, the key is
//! rotated, a second machine gets the same set — and none of that should mean
//! filling in six fields by hand per tool.

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
    let base = dirs::data_local_dir().context("kein LOCALAPPDATA / no local data directory")?;
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

pub fn save(services: &[Service]) -> Result<()> {
    let path = path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("{}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(services)?;
    std::fs::write(&path, text).with_context(|| format!("{}", path.display()))
}

/// A readable, stable id from a name — the same rule favourites use.
pub fn id_for(name: &str) -> String {
    let mut id: String = name
        .to_lowercase()
        .chars()
        .map(|c| match c.is_alphanumeric() {
            true => c,
            false => '_',
        })
        .collect();
    id.truncate(48);
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
    let mut fields = Vec::new();
    if let Some(settings) = settings
        && let spec::Settings::Text { field, .. } | spec::Settings::Fields { field, .. } =
            &tool.settings
    {
        fields.push(crate::favourites::Header {
            name: field.clone(),
            value: settings,
        });
    }

    Favourite {
        id: format!("{}__{}", service.id, id_for(&tool.summary)),
        name,
        icon: None,
        note: Some(tool.path.clone()),
        tool: Tool::Web(WebTool {
            mode: WebMode::Upload(Upload {
                endpoint: endpoint_for(service, tool),
                method: tool.method.clone(),
                body: UploadBody::Multipart {
                    field: tool.file_field.clone(),
                },
                headers: service.auth_header.clone().into_iter().collect(),
                fields,
                result: ResultAction::Save {
                    source: match service.result_path.trim().is_empty() {
                        true => ResultSource::Body,
                        false => ResultSource::Json {
                            path: service.result_path.clone(),
                        },
                    },
                    suffix: suffix.to_string(),
                },
            }),
            allow_insecure: service.allow_insecure,
            // The service was confirmed as a whole when it was added; asking
            // again per tool would be the same question a hundred times.
            confirmed: true,
        }),
    }
}

/// The full address of one tool: the origin of the description plus the path.
fn endpoint_for(service: &Service, tool: &spec::Tool) -> String {
    let origin = service
        .spec_url
        .split_once("://")
        .map(|(scheme, rest)| {
            let host = rest.split('/').next().unwrap_or(rest);
            format!("{scheme}://{host}")
        })
        .unwrap_or_else(|| service.spec_url.clone());
    format!("{origin}{}", tool.path)
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
    fn ids_stay_readable_and_usable_as_file_names() {
        assert_eq!(id_for("SnapOtter"), "snapotter");
        assert_eq!(id_for("Bild verkleinern!"), "bild_verkleinern_");
        assert_eq!(id_for("   "), "dienst");
    }
}
