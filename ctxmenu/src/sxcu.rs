//! Reading ShareX's own uploader files.
//!
//! ShareX has described custom uploaders as `.sxcu` files for years, and the
//! internet is full of them: a service that wants to be used writes one and
//! puts it on its documentation page. The format says the same things an
//! upload favourite does, under different names.
//!
//! | `.sxcu` | here |
//! |---|---|
//! | `RequestMethod` | the method |
//! | `RequestURL` plus `Parameters` | the endpoint, query string included |
//! | `Headers` | the header lines |
//! | `Body: MultipartFormData` | multipart |
//! | `Body: Binary` | the raw file |
//! | `FileFormName` | the field name |
//! | `Arguments` | the form fields beside the file |
//! | `URL` | where the answer names the result |
//!
//! # What is refused rather than approximated
//!
//! `Body: FormURLEncoded`, `JSON` and `XML` put the *content* somewhere rather
//! than attaching the file, which is the one thing this program does not do.
//! `{xml:…}` and `{regex:…}` read an answer this program cannot read, and
//! `{input}` and `{select:…}` ask the user a question mid-upload. Each of them
//! ends the import with a sentence naming the thing that was in the way,
//! because a favourite built around a field that was quietly dropped is a
//! favourite that fails at a service, once, in a menu, with no way back to the
//! cause.
//!
//! `DeletionURL` and `ThumbnailURL` are read and dropped on purpose: they name
//! a second and third address, and a favourite has one result.

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::favourites::{
    Favourite, Header, ResultAction, ResultSource, Tool, Upload, UploadBody, WebMode, WebTool,
};

/// The fields of the format, as far as they mean anything here.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Sxcu {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "RequestMethod")]
    request_method: String,
    #[serde(rename = "RequestURL")]
    request_url: String,
    #[serde(rename = "Parameters")]
    parameters: serde_json::Map<String, Value>,
    #[serde(rename = "Headers")]
    headers: serde_json::Map<String, Value>,
    #[serde(rename = "Body")]
    body: String,
    #[serde(rename = "FileFormName")]
    file_form_name: String,
    #[serde(rename = "Arguments")]
    arguments: serde_json::Map<String, Value>,
    #[serde(rename = "URL")]
    url: String,
}

/// Turns the text of a `.sxcu` file into a favourite.
pub fn read(text: &str) -> Result<Favourite> {
    let file: Sxcu = serde_json::from_str(text)
        .context("\x1eKeine lesbare .sxcu-Datei\x1fnot a readable .sxcu file\x1d")?;

    if file.request_url.trim().is_empty() {
        bail!("\x1eDie Datei nennt keine Adresse\x1fthe file names no address\x1d");
    }

    let method = match file.request_method.trim().to_ascii_uppercase().as_str() {
        "" | "POST" => "POST".to_string(),
        "PUT" => "PUT".to_string(),
        other => bail!(
            "\x1eMethode {other} wird hier nicht geschickt\
             \x1fmethod {other} is not something this sends\x1d"
        ),
    };

    let body = match file.body.trim() {
        "MultipartFormData" | "" => UploadBody::Multipart {
            field: match file.file_form_name.trim() {
                "" => "file".to_string(),
                field => field.to_string(),
            },
        },
        "Binary" => UploadBody::Raw,
        other => bail!(
            "\x1e{other} legt den Inhalt in ein Feld, statt die Datei anzuhängen — \
             das kann dieses Programm nicht\
             \x1f{other} puts the content into a field rather than attaching the file, \
             which this program cannot do\x1d"
        ),
    };

    let endpoint = with_parameters(file.request_url.trim(), &file.parameters);
    let result = result_of(&file.url)?;

    Ok(Favourite {
        id: String::new(),
        name: match file.name.trim() {
            "" => host_of(&endpoint).to_string(),
            name => name.to_string(),
        },
        icon: None,
        note: None,
        // A ShareX file is a service arriving by another door, and the
        // services tab is where it belongs afterwards.
        from: Some("sxcu".into()),
        tool: Tool::Web(WebTool {
            mode: WebMode::Upload(Box::new(Upload {
                endpoint,
                method,
                body,
                headers: pairs(&file.headers),
                fields: pairs(&file.arguments),
                poll: None,
                result,
            })),
            allow_insecure: false,
            confirmed: false,
        }),
    })
}

/// `Parameters` are a query string ShareX keeps apart. Here they belong to the
/// address, because the address field takes a whole one.
fn with_parameters(url: &str, parameters: &serde_json::Map<String, Value>) -> String {
    if parameters.is_empty() {
        return url.to_string();
    }

    let query: Vec<String> = parameters
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                crate::webtool::encode(name),
                crate::webtool::encode(&as_text(value))
            )
        })
        .collect();

    let separator = match url.contains('?') {
        true => '&',
        false => '?',
    };
    format!("{url}{separator}{}", query.join("&"))
}

fn pairs(map: &serde_json::Map<String, Value>) -> Vec<Header> {
    map.iter()
        .map(|(name, value)| Header {
            name: name.clone(),
            value: as_text(value),
        })
        .collect()
}

fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// What ShareX writes into `URL`, read as a result.
///
/// Both spellings of the placeholders are accepted: ShareX 13 and later use
/// `{json:…}`, everything older uses `$json:…$`, and files of both ages are in
/// circulation.
fn result_of(url: &str) -> Result<ResultAction> {
    let url = url.trim();

    if url.is_empty() {
        // Nothing said about the answer at all, which is what an uploader for
        // a service that answers with the finished file looks like.
        return Ok(ResultAction::Save {
            source: ResultSource::Body,
            suffix: ".neu".into(),
            extension: String::new(),
        });
    }

    let inner = url
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .or_else(|| {
            url.strip_prefix('$')
                .and_then(|rest| rest.strip_suffix('$'))
        });

    match inner {
        Some(marker) => {
            let marker = marker.trim();
            if let Some(path) = marker.strip_prefix("json:") {
                return Ok(ResultAction::Open {
                    source: ResultSource::Json {
                        path: path.trim().to_string(),
                    },
                });
            }
            if marker.eq_ignore_ascii_case("response") {
                return Ok(ResultAction::Open {
                    source: ResultSource::Body,
                });
            }
            if marker.eq_ignore_ascii_case("responseurl") {
                return Ok(ResultAction::Open {
                    source: ResultSource::Location,
                });
            }
            bail!(
                "\x1eDie Adresse der Antwort steht als {{{marker}}} da, und das liest \
                 dieses Programm nicht\
                 \x1fthe answer's address is written as {{{marker}}}, which this program \
                 does not read\x1d"
            )
        }
        // Something built around a placeholder rather than being one:
        // `https://x.example/{json:id}`. That is exactly the shape `Built`
        // takes, once the marker is written the way it is written here.
        None => match url.contains("{json:") || url.contains("$json:") {
            true => Ok(ResultAction::Open {
                source: ResultSource::Built {
                    url: url
                        .replace("{json:", "{")
                        .replace("$json:", "{")
                        .replace('$', "}"),
                },
            }),
            false => bail!(
                "\x1eDie Adresse der Antwort ist weder ein Feld noch eine Vorlage\
                 \x1fthe answer's address is neither a field nor a template\x1d: {url}"
            ),
        },
    }
}

fn host_of(url: &str) -> &str {
    url.split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example from ShareX's own documentation, read 2026-08-21.
    const EXAMPLE: &str = r#"{
      "Version": "17.0.0",
      "Name": "Example",
      "DestinationType": "ImageUploader, FileUploader",
      "RequestMethod": "POST",
      "RequestURL": "https://example.com/upload.php",
      "Parameters": { "album": "public" },
      "Headers": { "Authorization": "Bearer abc" },
      "Body": "MultipartFormData",
      "Arguments": { "quality": "90" },
      "FileFormName": "source",
      "URL": "{json:url}",
      "ThumbnailURL": "{json:thumbnail_url}",
      "DeletionURL": "{json:deletion_url}",
      "ErrorMessage": "{json:error}"
    }"#;

    fn upload_of(favourite: &Favourite) -> Upload {
        match &favourite.tool {
            Tool::Web(WebTool {
                mode: WebMode::Upload(upload),
                ..
            }) => (**upload).clone(),
            other => panic!("expected an upload, got {other:?}"),
        }
    }

    #[test]
    fn the_documented_example_reads_field_for_field() {
        let favourite = read(EXAMPLE).expect("the documented example has to read");
        assert_eq!(favourite.name, "Example");
        assert!(favourite.id.is_empty(), "the list hands out the id");

        assert_eq!(
            favourite.from.as_deref(),
            Some("sxcu"),
            "a file read here is a service, and the services tab shows it"
        );

        let upload = upload_of(&favourite);
        assert_eq!(upload.method, "POST");
        assert_eq!(
            upload.endpoint, "https://example.com/upload.php?album=public",
            "Parameters belong to the address here"
        );
        assert_eq!(
            upload.body,
            UploadBody::Multipart {
                field: "source".into()
            }
        );
        assert_eq!(
            upload.headers,
            vec![Header {
                name: "Authorization".into(),
                value: "Bearer abc".into()
            }]
        );
        assert_eq!(
            upload.fields,
            vec![Header {
                name: "quality".into(),
                value: "90".into()
            }]
        );
        assert_eq!(
            upload.result,
            ResultAction::Open {
                source: ResultSource::Json { path: "url".into() }
            }
        );
    }

    #[test]
    fn the_older_spelling_of_a_placeholder_reads_too() {
        let old = r#"{ "RequestURL": "https://x.example/u", "Body": "Binary",
                       "URL": "$json:data.link$" }"#;
        let upload = upload_of(&read(old).expect("reads"));

        assert_eq!(upload.body, UploadBody::Raw);
        assert_eq!(
            upload.result,
            ResultAction::Open {
                source: ResultSource::Json {
                    path: "data.link".into()
                }
            }
        );
        assert_eq!(
            read(old).unwrap().name,
            "x.example",
            "a file without a name is named after its host, as ShareX does"
        );
    }

    #[test]
    fn an_address_around_a_field_becomes_a_template() {
        let file = r#"{ "RequestURL": "https://x.example/u",
                        "URL": "https://x.example/p/{json:data.id}" }"#;
        let upload = upload_of(&read(file).expect("reads"));

        assert_eq!(
            upload.result,
            ResultAction::Open {
                source: ResultSource::Built {
                    url: "https://x.example/p/{data.id}".into()
                }
            }
        );
    }

    /// The refusals. Each of these is a file somebody will try, and each ends
    /// with a sentence rather than with a favourite that fails later.
    #[test]
    fn what_cannot_be_carried_over_is_refused_by_name() {
        let cases = [
            (
                r#"{ "RequestURL": "https://x.example/u", "Body": "JSON", "URL": "{json:url}" }"#,
                "JSON",
            ),
            (
                r#"{ "RequestURL": "https://x.example/u", "Body": "FormURLEncoded", "URL": "{json:url}" }"#,
                "FormURLEncoded",
            ),
            (
                r#"{ "RequestURL": "https://x.example/u", "URL": "{xml://Response/URL}" }"#,
                "xml",
            ),
            (
                r#"{ "RequestURL": "https://x.example/u", "URL": "{regex:1}" }"#,
                "regex",
            ),
        ];

        for (text, expected) in cases {
            let error = read(text).expect_err("has to be refused");
            let message = format!("{error:#}");
            assert!(
                message.contains(expected),
                "the refusal has to name {expected}: {message}"
            );
        }

        assert!(
            read("{ \"Body\": \"MultipartFormData\" }").is_err(),
            "a file without an address is not an uploader"
        );
        assert!(read("not json at all").is_err());
    }

    #[test]
    fn a_file_that_says_nothing_about_the_answer_saves_it() {
        let file = r#"{ "RequestURL": "https://x.example/u", "Body": "Binary" }"#;
        let upload = upload_of(&read(file).expect("reads"));

        assert!(
            matches!(
                upload.result,
                ResultAction::Save {
                    source: ResultSource::Body,
                    ..
                }
            ),
            "an uploader that says nothing answers with the file itself"
        );
    }
}
