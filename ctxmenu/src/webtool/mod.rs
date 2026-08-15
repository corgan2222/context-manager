//! Was passiert, wenn ein Webtool-Eintrag angeklickt wird.
//!
//! A context menu entry can only start a program with a file name. It cannot
//! upload anything, and a URL cannot fetch a local file — no browser permits a
//! page to read `C:\bild.png` just because the address says so. So the entry
//! starts *this* application with `--favourite <id> "%1"`, and everything the
//! chosen tool needs happens here.
//!
//! Three ways, because real web tools differ:
//!
//! | Betriebsart | Was passiert | Wofür |
//! |---|---|---|
//! | [`Open`](crate::favourites::WebMode::Open) | Adresse aus Platzhaltern bauen und öffnen | Suche, Nachschlagewerk — die Datei bleibt hier |
//! | [`Clipboard`](crate::favourites::WebMode::Clipboard) | Datei in die Zwischenablage, dann Seite öffnen | Squoosh und alles ohne Schnittstelle: Strg+V genügt |
//! | [`Upload`](crate::favourites::WebMode::Upload) | Datei per HTTP schicken, Ergebnis abholen | Tools mit echtem Endpunkt |
//!
//! Only the third one actually transfers the file, and only that one asks
//! first.

pub mod http;
pub mod shell;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::favourites::{ResultAction, ResultSource, Tool, UploadBody, WebMode};

/// The argument that puts this program into web tool mode.
///
/// A constant rather than a literal because it appears in three places that
/// must agree: the command line written into the registry, the interception in
/// `main`, and the help text. Two of them silently doing nothing is exactly
/// the failure this prevents.
pub const RUN_ARG: &str = "--favourite";

/// Runs a favourite against one file. The whole `--favourite` mode.
pub fn run(id: &str, file: &Path) -> Result<String> {
    let favourite = crate::favourites::find(id)?;

    if !file.exists() {
        bail!("Datei nicht gefunden / file not found: {}", file.display());
    }

    let Tool::Web(web) = &favourite.tool else {
        // A program favourite has the executable in its own command line and
        // never comes through here. Reaching this point means an entry was
        // written from a favourite that has since been turned into a program.
        bail!("„{}“ ist kein Webtool / is not a web tool", favourite.name);
    };

    match &web.mode {
        WebMode::Open { url } => {
            let address = fill(url, file);
            shell::open(&address)?;
            Ok(format!("Geöffnet / opened: {address}"))
        }

        WebMode::Clipboard { url } => {
            // Clipboard first: the browser takes a moment to come up, and by
            // then the file is already there to paste.
            shell::copy_file_to_clipboard(file)?;
            let address = fill(url, file);
            shell::open(&address)?;
            Ok(format!(
                "{} liegt in der Zwischenablage — im Browser Strg+V drücken. / \
                 on the clipboard; press Ctrl+V in the browser.",
                file.file_name().unwrap_or_default().to_string_lossy()
            ))
        }

        WebMode::Upload(upload) => {
            // The one place in this program where data leaves the machine.
            // Asked once per favourite and recorded, so it is a decision about
            // this service rather than a habit of clicking yes.
            if !web.confirmed {
                let size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
                let agreed = shell::ask(
                    "Datei senden? / Send the file?",
                    &format!(
                        "„{}“ schickt diese Datei an einen fremden Dienst:\n\n\
                         Datei: {}\n\
                         Größe: {} KB\n\
                         Ziel:  {}\n\n\
                         Einverstanden? Diese Frage kommt für dieses Werkzeug nur einmal.\n\n\
                         — — —\n\n\
                         \"{}\" sends this file to an external service.\n\
                         Agree? You will be asked once per tool.",
                        favourite.name,
                        file.display(),
                        size.div_ceil(1024),
                        host_of(&upload.endpoint),
                        favourite.name,
                    ),
                );

                if !agreed {
                    return Ok("Nichts gesendet / nothing was sent".into());
                }

                // Remember the answer, but never let a failure to write it
                // stop the upload the user just agreed to.
                let mut remembered = favourite.clone();
                if let Tool::Web(web) = &mut remembered.tool {
                    web.confirmed = true;
                }
                let _ = crate::favourites::update(remembered);
            }

            let bytes = std::fs::read(file).with_context(|| format!("{}", file.display()))?;
            let name = file
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let request = match &upload.body {
                UploadBody::Raw => http::Request::raw(bytes, mime_for(file)),
                UploadBody::Multipart { field } => {
                    http::Request::multipart(field, &name, bytes, mime_for(file), &upload.fields)
                }
            };

            let answer = http::send(&upload.endpoint, &upload.method, &upload.headers, request)
                .with_context(|| format!("{} {}", upload.method, upload.endpoint))?;

            apply_result(&upload.result, &answer, file, &upload.endpoint)
        }
    }
}

/// Makes an address the service handed back usable on its own.
///
/// Services answer with a path at least as often as with a full address:
/// SnapOtter returns `"downloadUrl": "/api/v1/download/…"`, and taken literally
/// that is not somewhere anything can be fetched from. Resolved against the
/// endpoint it came from, it is.
///
/// Only two cases, because only two occur: an address that already carries a
/// scheme is returned untouched, and one starting with `/` is hung under the
/// origin of the endpoint. A relative path without the leading slash would have
/// to guess how much of the endpoint's own path to keep, and guessing at where
/// to send a request is not something this should do quietly.
fn absolute(address: &str, endpoint: &str) -> Result<String> {
    if address.contains("://") {
        return Ok(address.to_string());
    }
    if !address.starts_with('/') {
        bail!("Weder Adresse noch Pfad / neither an address nor a path: {address}");
    }

    let (scheme, rest) = endpoint
        .split_once("://")
        .context("Endpunkt ohne Schema / endpoint without a scheme")?;
    let host = rest.split('/').next().unwrap_or(rest);
    Ok(format!("{scheme}://{host}{address}"))
}

/// Just the host of an address, for the question before sending.
///
/// The whole endpoint including path and key would be noise in a dialog whose
/// only job is to make one thing plain: where this file is going.
fn host_of(url: &str) -> &str {
    url.split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
}

/// Did the service take the job rather than do it?
///
/// `202` is the polite signal, and a body carrying `"async": true` is the one
/// that turns up when the description said `200`. Measured on SnapOtter
/// (2026-08-15): `image/sharpening` lists only `200` in its own OpenAPI
/// description and answers `202 {"jobId": …, "async": true}` — so reading the
/// description is not enough, and a saved file would be a JSON stub with a
/// picture's name.
fn took_the_job(answer: &http::Answer) -> bool {
    if answer.status == 202 {
        return true;
    }
    serde_json::from_slice::<serde_json::Value>(&answer.body)
        .ok()
        .and_then(|value| value.get("async").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// Turns the answer into something the user can see or keep.
fn apply_result(
    action: &ResultAction,
    answer: &http::Answer,
    file: &Path,
    endpoint: &str,
) -> Result<String> {
    // Before anything is fetched or written: what came back is a receipt, not
    // a result, and every path below would make a mess of it.
    if took_the_job(answer) && !matches!(action, ResultAction::Report) {
        bail!(
            "Der Dienst arbeitet im Hintergrund und hat nur eine Auftragsnummer geschickt. \
             Dieses Werkzeug kann das Ergebnis noch nicht abholen. / the service queued the \
             job and answered with an id only; fetching that result is not supported yet"
        );
    }

    match action {
        ResultAction::Report => Ok(format!(
            "Antwort {} / status {}, {} Bytes",
            answer.status,
            answer.status,
            answer.body.len()
        )),

        ResultAction::Open { source } => {
            let address = absolute(&locate(source, answer)?, endpoint)?;
            shell::open(&address)?;
            Ok(format!("Ergebnis geöffnet / result opened: {address}"))
        }

        ResultAction::Save { source, suffix } => {
            let bytes = match source {
                ResultSource::Body => answer.body.clone(),
                other => {
                    // The service answered with an address rather than the
                    // file; fetching it is the other half of the job.
                    let address = absolute(&locate(other, answer)?, endpoint)?;
                    http::download(&address).with_context(|| address.clone())?
                }
            };

            if bytes.is_empty() {
                bail!("Antwort ohne Inhalt / empty answer, nothing to save");
            }

            let target = free_name(file, suffix);
            std::fs::write(&target, &bytes).with_context(|| format!("{}", target.display()))?;
            Ok(format!(
                "Gespeichert / saved: {} ({} Bytes)",
                target.display(),
                bytes.len()
            ))
        }
    }
}

/// Digs the result address out of the answer.
fn locate(source: &ResultSource, answer: &http::Answer) -> Result<String> {
    match source {
        ResultSource::Body => {
            let text = String::from_utf8_lossy(&answer.body).trim().to_string();
            if text.starts_with("http") {
                Ok(text)
            } else {
                bail!("Antwort ist keine Adresse / the answer is not an address")
            }
        }
        ResultSource::Location => answer
            .header("Location")
            .map(str::to_string)
            .context("Kein Location-Kopf in der Antwort / no Location header"),
        ResultSource::Json { path } => {
            let value: serde_json::Value = serde_json::from_slice(&answer.body)
                .context("Antwort ist kein JSON / the answer is not JSON")?;
            json_path(&value, path)
                .with_context(|| format!("Kein Feld {path} in der Antwort / no such field"))
        }
    }
}

/// Follows a dotted path such as `output.url` through a JSON value.
///
/// Deliberately not a full JSON Pointer: dots are what people write, and every
/// service this was built against nests two levels at most.
fn json_path(value: &serde_json::Value, path: &str) -> Option<String> {
    let mut current = value;
    for step in path.split('.') {
        current = match current {
            serde_json::Value::Object(map) => map.get(step)?,
            serde_json::Value::Array(items) => items.get(step.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }

    match current {
        serde_json::Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

/// Fills the placeholders of an address template.
///
/// Everything substituted is percent-encoded: a file called `Bild & Text.png`
/// would otherwise end the query string early and quietly change what the tool
/// is asked to do.
pub fn fill(template: &str, file: &Path) -> String {
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    let stem = file.file_stem().unwrap_or_default().to_string_lossy();
    let ext = file
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir = file
        .parent()
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default();
    let path = file.to_string_lossy();

    template
        .replace("{name}", &encode(&name))
        .replace("{stem}", &encode(&stem))
        .replace("{ext}", &encode(&ext))
        .replace("{dir}", &encode(&dir))
        .replace("{fileurl}", &file_url(file))
        .replace("{path}", &encode(&path))
}

/// `file:///C:/Ordner/Bild.png`
fn file_url(file: &Path) -> String {
    let mut url = String::from("file:///");
    for character in file.to_string_lossy().chars() {
        match character {
            '\\' => url.push('/'),
            ':' | '/' => url.push(character),
            other => url.push_str(&encode(&other.to_string())),
        }
    }
    url
}

/// Percent-encoding for everything that is not unreserved in RFC 3986.
///
/// Hand-written rather than pulled in: it is twenty lines, and a dependency
/// that only encodes URLs would still have to be updated forever.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// A name next to the original that is not taken.
///
/// `bild.png` with `.min` becomes `bild.min.png`, and if that exists,
/// `bild.min_2.png`. Overwriting is never on offer: the original is the
/// user's, and a web tool that answers with rubbish must not be able to
/// destroy it.
pub fn free_name(original: &Path, suffix: &str) -> PathBuf {
    let directory = original.parent().unwrap_or_else(|| Path::new("."));
    let stem = original.file_stem().unwrap_or_default().to_string_lossy();
    let extension = original
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let suffix = suffix.trim();

    let mut candidate = directory.join(format!("{stem}{suffix}{extension}"));
    let mut counter = 2;
    while candidate.exists() {
        candidate = directory.join(format!("{stem}{suffix}_{counter}{extension}"));
        counter += 1;
    }
    candidate
}

/// A content type for the file, good enough for a form field.
///
/// Not a lookup of the whole registry: the services this talks to either know
/// the type from the file name or do not care, and `application/octet-stream`
/// is the honest answer for anything else.
pub fn mime_for(file: &Path) -> &'static str {
    let extension = file
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "json" => "application/json",
        "txt" | "md" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_question_before_sending_names_the_host_and_nothing_else() {
        // A key in the query string has no business in a dialog whose job is
        // to say where the file goes.
        assert_eq!(
            host_of("https://api.tinify.com/shrink?key=geheim"),
            "api.tinify.com"
        );
        assert_eq!(
            host_of("http://192.168.1.10:8080/upload"),
            "192.168.1.10:8080"
        );
        assert_eq!(host_of("api.example.org"), "api.example.org");
    }

    #[test]
    fn placeholders_are_filled_and_encoded() {
        let file = Path::new(r"C:\Bilder\Sonne & Meer.png");

        assert_eq!(
            fill("https://x.example/?q={stem}", file),
            "https://x.example/?q=Sonne%20%26%20Meer"
        );
        assert_eq!(
            fill("https://x.example/?n={name}&e={ext}", file),
            "https://x.example/?n=Sonne%20%26%20Meer.png&e=png"
        );

        // The ampersand must not survive raw: it would end the parameter and
        // add one the tool never asked for.
        assert!(!fill("https://x.example/?q={name}", file).contains("&M"));
    }

    #[test]
    fn a_file_url_keeps_its_slashes_and_drive_letter() {
        let url = file_url(Path::new(r"C:\Bilder\Sonne & Meer.png"));
        assert!(url.starts_with("file:///C:/Bilder/"), "got {url}");
        assert!(url.contains("%20"), "spaces must be encoded: {url}");
        assert!(!url.contains('\\'), "no backslashes in a URL: {url}");
    }

    #[test]
    fn a_template_without_placeholders_is_left_alone() {
        let file = Path::new(r"C:\a\b.png");
        assert_eq!(fill("https://squoosh.app", file), "https://squoosh.app");
    }

    #[test]
    fn the_result_never_overwrites_the_original() {
        let directory = std::env::temp_dir().join("ctxmenu_webtool_name_test");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temp directory");

        let original = directory.join("bild.png");
        std::fs::write(&original, b"x").expect("write");

        let first = free_name(&original, ".min");
        assert_eq!(first.file_name().unwrap(), "bild.min.png");
        assert_ne!(first, original, "the original must never be the target");

        std::fs::write(&first, b"y").expect("write");
        let second = free_name(&original, ".min");
        assert_eq!(second.file_name().unwrap(), "bild.min_2.png");

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_file_without_an_extension_still_gets_a_usable_name() {
        let name = free_name(Path::new(r"C:\a\LIESMICH"), ".neu");
        assert_eq!(name.file_name().unwrap(), "LIESMICH.neu");
    }

    #[test]
    fn a_dotted_path_finds_the_address_in_the_answer() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{"output":{"url":"https://api.example/out/1","size":842},"input":{"size":9000}}"#,
        )
        .expect("json");

        assert_eq!(
            json_path(&value, "output.url").as_deref(),
            Some("https://api.example/out/1")
        );
        assert_eq!(json_path(&value, "output.size").as_deref(), Some("842"));
        assert_eq!(json_path(&value, "output.gibtsnicht"), None);
        assert_eq!(json_path(&value, "input"), Some(r#"{"size":9000}"#.into()));
    }

    #[test]
    fn a_queued_job_is_reported_rather_than_saved_as_a_picture() {
        let file = Path::new("bild.png");
        let save = ResultAction::Save {
            source: ResultSource::Body,
            suffix: ".neu".into(),
        };

        // The polite signal.
        let accepted = http::Answer {
            status: 202,
            headers: Vec::new(),
            body: br#"{"jobId":"abc"}"#.to_vec(),
        };
        assert!(apply_result(&save, &accepted, file, "http://x/y").is_err());

        // And the one that turns up when the description promised 200:
        // measured on a real service, which answers this to a tool its own
        // OpenAPI lists as synchronous.
        let lying = http::Answer {
            status: 200,
            headers: Vec::new(),
            body: br#"{"jobId":"abc","async":true}"#.to_vec(),
        };
        assert!(apply_result(&save, &lying, file, "http://x/y").is_err());

        // Reporting is still allowed: that is the mode for looking at what a
        // service actually says.
        assert!(apply_result(&ResultAction::Report, &accepted, file, "http://x/y").is_ok());

        // An ordinary answer is untouched by any of this.
        let ordinary = http::Answer {
            status: 200,
            headers: Vec::new(),
            body: b"nicht wirklich ein PNG".to_vec(),
        };
        let directory = std::env::temp_dir().join("ctxmenu_async_test");
        let _ = std::fs::create_dir_all(&directory);
        let target = directory.join("bild.png");
        assert!(apply_result(&save, &ordinary, &target, "http://x/y").is_ok());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_path_in_the_answer_is_resolved_against_the_endpoint() {
        // Measured against a real service on 2026-08-15: SnapOtter answers a
        // compress request with `"downloadUrl": "/api/v1/download/…"`, which
        // taken literally is nowhere.
        let endpoint = "http://192.168.2.11:1349/api/v1/tools/image/compress";
        assert_eq!(
            absolute("/api/v1/download/abc/test_compress.png", endpoint).unwrap(),
            "http://192.168.2.11:1349/api/v1/download/abc/test_compress.png"
        );

        // A full address is the service's own business and stays untouched,
        // host and scheme included.
        let full = "https://cdn.example/out/1.png";
        assert_eq!(absolute(full, endpoint).unwrap(), full);

        // Anything else is refused rather than guessed at: how much of the
        // endpoint's path to keep is not something to decide quietly when the
        // answer is where a request goes next.
        assert!(absolute("out/1.png", endpoint).is_err());
        assert!(absolute("/x", "kein-schema/api").is_err());
    }

    #[test]
    fn the_content_type_follows_the_extension_case_insensitively() {
        assert_eq!(mime_for(Path::new("a.PNG")), "image/png");
        assert_eq!(mime_for(Path::new("a.jpeg")), "image/jpeg");
        assert_eq!(
            mime_for(Path::new("a.unbekannt")),
            "application/octet-stream"
        );
        assert_eq!(mime_for(Path::new("ohne")), "application/octet-stream");
    }
}
