//! What happens when a web tool entry is clicked.
//!
//! A context menu entry can only start a program with a file name. It cannot
//! upload anything, and a URL cannot fetch a local file — no browser permits a
//! page to read `C:\bild.png` just because the address says so. So the entry
//! starts *this* application with `--favourite <id> "%1"`, and everything the
//! chosen tool needs happens here.
//!
//! Three ways, because real web tools differ:
//!
//! | Mode | What happens | What for |
//! |---|---|---|
//! | [`Open`](crate::favourites::WebMode::Open) | build the address from the placeholders and open it | search, reference works — the file stays here |
//! | [`Clipboard`](crate::favourites::WebMode::Clipboard) | file onto the clipboard, then open the page | Squoosh and everything without an interface: Ctrl+V is enough |
//! | [`Upload`](crate::favourites::WebMode::Upload) | send the file over HTTP, fetch the result back | tools with a real endpoint |
//!
//! Only the third one actually transfers the file, and only that one asks
//! first.

pub mod http;
pub mod shell;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use crate::favourites::{Poll, ResultAction, ResultSource, Tool, Upload, UploadBody, WebMode};

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
        bail!(
            "\x1eDatei nicht gefunden\x1ffile not found\x1d: {}",
            file.display()
        );
    }

    let Tool::Web(web) = &favourite.tool else {
        // A program favourite has the executable in its own command line and
        // never comes through here. Reaching this point means an entry was
        // written from a favourite that has since been turned into a program.
        bail!(
            "„{}“ \x1eist kein Webtool\x1fis not a web tool\x1d",
            favourite.name
        );
    };

    match &web.mode {
        WebMode::Open { url } => {
            let address = fill(url, file);
            shell::open(&address)?;
            Ok(format!("\x1eGeöffnet\x1fopened\x1d: {address}"))
        }

        WebMode::Clipboard { url } => {
            // Clipboard first: the browser takes a moment to come up, and by
            // then the file is already there to paste.
            shell::copy_file_to_clipboard(file)?;
            let address = fill(url, file);
            shell::open(&address)?;
            Ok(format!(
                "{} \x1eliegt in der Zwischenablage — im Browser Strg+V drücken.\
                 \x1fis on the clipboard; press Ctrl+V in the browser.\x1d",
                file.file_name().unwrap_or_default().to_string_lossy()
            ))
        }

        WebMode::Upload(upload) => {
            // Before the question rather than after it: agreeing to send a file
            // over a connection this tool was never allowed to use would be an
            // agreement to nothing, and the answer is remembered.
            permitted(&upload.endpoint, web.allow_insecure)?;

            // The one place in this program where data leaves the machine.
            // Asked once per favourite and recorded, so it is a decision about
            // this service rather than a habit of clicking yes.
            if !web.confirmed {
                let size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
                let agreed = shell::ask(
                    "\x1eDatei senden?\x1fSend the file?\x1d",
                    &format!(
                        "„{name}“ \x1eschickt diese Datei an einen fremden Dienst\
                         \x1fsends this file to an external service\x1d:\n\n\
                         \x1eDatei\x1fFile\x1d: {file}\n\
                         \x1eGröße\x1fSize\x1d: {kilobytes} KB\n\
                         \x1eZiel\x1fTo\x1d: {host}\n\n\
                         \x1eEinverstanden? Diese Frage kommt für dieses Werkzeug nur einmal.\
                         \x1fAgree? You will be asked once per tool.\x1d",
                        name = favourite.name,
                        file = file.display(),
                        kilobytes = size.div_ceil(1024),
                        host = host_of(&upload.endpoint),
                    ),
                );

                if !agreed {
                    return Ok("\x1eNichts gesendet\x1fnothing was sent\x1d".into());
                }

                // Remember the answer, but never let a failure to write it
                // stop the upload the user just agreed to.
                //
                // The one field, and not the whole favourite: this process was
                // started by a right-click and the window may well be open
                // beside it, with a favourite half renamed. Writing back the
                // copy read at the top of this function would take that rename
                // with it -- and that copy is now as old as the dialog above
                // stood on screen.
                let _ = crate::favourites::remember_consent(&favourite.id);
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

            apply_result(upload, &answer, file, web.allow_insecure)
        }
    }
}

/// Whether this program may act on an address at all.
///
/// Two rules in one place, because every address this sends a file to, fetches
/// a result from, or opens on the strength of an answer has to pass both:
///
/// * It is `http://` or `https://` and nothing else. `file:` is refused here
///   even though [`fill`] builds one for `{fileurl}` — that placeholder is this
///   program pointing a page at the file the user clicked, whereas
///   `{"url": "file:///C:/Users/…/setup.exe"}` in an answer is a foreign
///   service picking a program on this machine to have started.
/// * Unencrypted `http://` only where the user said so. The tick box called
///   "allow unencrypted http://" used to be read by
///   [`crate::favourites::Favourite::problems`] alone, which advises and never
///   refuses, so the upload itself went out in the clear regardless. A service
///   in the local network is exactly what the tick box is for, and with it set
///   nothing here changes.
fn permitted(address: &str, allow_insecure: bool) -> Result<()> {
    if address.starts_with("https://") {
        return Ok(());
    }

    if address.starts_with("http://") {
        if allow_insecure {
            return Ok(());
        }
        bail!(
            "\x1eUnverschlüsselte Adresse: die Datei ginge im Klartext durchs Netz. \
             Für dieses Werkzeug „unverschlüsseltes http:// erlauben“ ankreuzen, \
             wenn das so gewollt ist.\x1funencrypted address; the file would travel \
             in the clear. Tick “allow unencrypted http://” for this tool if that \
             is what you want.\x1d: {address}"
        );
    }

    bail!("\x1eKeine Web-Adresse\x1fnot a web address\x1d: {address}")
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
///
/// Which scheme is acceptable is deliberately not decided here: [`permitted`]
/// does that for both callers afterwards, so the rule sits in one place rather
/// than being split between resolving an address and using it.
fn absolute(address: &str, endpoint: &str) -> Result<String> {
    if address.contains("://") {
        return Ok(address.to_string());
    }
    if !address.starts_with('/') {
        bail!("\x1eWeder Adresse noch Pfad\x1fneither an address nor a path\x1d: {address}");
    }

    let (scheme, rest) = endpoint
        .split_once("://")
        .context("\x1eEndpunkt ohne Schema\x1fendpoint without a scheme\x1d")?;
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
    upload: &Upload,
    answer: &http::Answer,
    file: &Path,
    allow_insecure: bool,
) -> Result<String> {
    let endpoint = &upload.endpoint;

    // Before anything is fetched or written: what came back may be a receipt
    // rather than a result, and every path below would make a mess of it —
    // `Save` would write the receipt out under a picture's name. Asking after
    // the job first turns it back into the answer the other branches expect.
    let job = match took_the_job(answer) && !matches!(upload.result, ResultAction::Report) {
        false => None,
        true => Some(awaited(upload, answer, allow_insecure)?),
    };

    match &upload.result {
        ResultAction::Report => Ok(format!(
            "\x1eAntwort\x1fstatus\x1d {}, {} Bytes",
            answer.status,
            answer.body.len()
        )),

        ResultAction::Open { source } => {
            let address = match &job {
                // Already resolved and already allowed: it came out of the
                // service's own progress and went through the same checks.
                Some(job) => job.address.clone(),
                None => {
                    let address = absolute(&locate(source, answer)?, endpoint)?;
                    permitted(&address, allow_insecure)?;
                    address
                }
            };
            shell::open_from_service(&address)?;
            Ok(format!(
                "{}\x1eErgebnis geöffnet\x1fresult opened\x1d: {address}",
                took_a_while(&job)
            ))
        }

        ResultAction::Save { source, suffix } => {
            let bytes = match (&job, source) {
                (Some(job), _) => {
                    http::download(&job.address).with_context(|| job.address.clone())?
                }
                (None, ResultSource::Body) => answer.body.clone(),
                (None, other) => {
                    // The service answered with an address rather than the
                    // file; fetching it is the other half of the job.
                    let address = absolute(&locate(other, answer)?, endpoint)?;
                    permitted(&address, allow_insecure)?;
                    http::download(&address).with_context(|| address.clone())?
                }
            };

            if bytes.is_empty() {
                bail!("\x1eAntwort ohne Inhalt\x1fempty answer\x1d, nothing to save");
            }

            let target = free_name(file, suffix);
            std::fs::write(&target, &bytes).with_context(|| format!("{}", target.display()))?;
            Ok(format!(
                "{}\x1eGespeichert\x1fsaved\x1d: {} ({} Bytes)",
                took_a_while(&job),
                target.display(),
                bytes.len()
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Jobs the service only took in.
// ---------------------------------------------------------------------------
//
// Measured against SnapOtter on 2026-08-16: six identical uploads of the same
// picture, four of which came back as `202 {"jobId": …, "async": true}` and two
// as the finished result. It is a property of how busy the service is, not of
// the endpoint, so it cannot be decided when the favourite is made — only when
// the answer arrives.
//
// What the service then wants is one `GET` on a path of its own, answered as
// Server-Sent Events:
//
// ```text
// data: {"jobId":"3488…","phase":"complete","percent":100,
//        "result":{"downloadUrl":"/api/v1/download/3488…/bild.png"},"type":"single"}
// ```
//
// — the tool's ordinary answer, wrapped in `result`. Which is why a favourite
// that already says where the ordinary answer names its file needs to say
// nothing more.

/// How long a queued job is given before this stops waiting.
///
/// Two minutes: the picture this was measured with takes eight seconds, and a
/// service that has not finished in fifteen times that is not busy but stuck.
const PATIENCE: Duration = Duration::from_secs(120);

/// How long one ask may hold the stream open before it is asked again.
///
/// The connection stays open for as long as the job runs, so without this the
/// waiting would happen inside a single read with no way to give up.
const ONE_ASK: Duration = Duration::from_secs(30);

/// The gap between two asks. Long enough not to hammer a service that is
/// already busy, short enough that a finished job is noticed at once.
const BETWEEN_ASKS: Duration = Duration::from_millis(1_500);

/// A job the service has finished, and where its result stands.
struct Finished {
    /// A whole address, resolved against the endpoint and already allowed.
    address: String,
    waited: Duration,
}

/// Waits for a job the service took in rather than did.
fn awaited(upload: &Upload, receipt: &http::Answer, allow_insecure: bool) -> Result<Finished> {
    let poll = match &upload.poll {
        Some(poll) => poll.clone(),
        // Nothing written down, which is every favourite made before this
        // existed. The service describes itself and its description says where
        // it takes questions, so ask it — that beats telling the user their
        // tool box has to be built again.
        None => discovered(&upload.endpoint, allow_insecure)?,
    };

    let receipt: serde_json::Value = serde_json::from_slice(&receipt.body).context(
        "\x1eDer Dienst hat den Auftrag eingereiht, aber keine lesbare Antwort geschickt\
         \x1fthe service queued the job and answered with something that is not JSON\x1d",
    )?;
    let id = json_path(&receipt, &poll.job)
        .filter(|id| !id.trim().is_empty())
        .with_context(|| {
            format!(
                "\x1eKeine Auftragsnummer unter {field} in der Antwort\
                 \x1fno job id at {field} in the answer\x1d",
                field = poll.job
            )
        })?;

    let asking = absolute(&asking_after(&poll.path, &id)?, &upload.endpoint)?;
    permitted(&asking, allow_insecure)?;

    let where_it_stands = frame_path(&poll, &upload.result).context(
        "\x1eDer Dienst hat den Auftrag eingereiht, und dieses Werkzeug weiß nicht, wo \
         die fertige Datei in der Fortschrittsmeldung steht — dafür braucht der Favorit \
         ein Feld im JSON, nicht die Antwort selbst.\x1fthe service queued the job, and \
         this tool has no idea where the finished file is named in a progress frame — \
         that needs a favourite whose result is a field in the JSON rather than the \
         answer itself\x1d",
    )?;

    // In `--favourite` mode there is no console and no window, so this is seen
    // only when the same favourite is run from a terminal. It is still the one
    // place that says the program is waiting rather than hanging; what happened
    // reaches the click through the message at the end.
    crate::outln!(
        "\x1eDer Dienst arbeitet, warte auf das Ergebnis …\
         \x1fthe service is working; waiting for the result …\x1d"
    );

    let started = Instant::now();
    loop {
        let left = PATIENCE.saturating_sub(started.elapsed());
        if left.is_zero() {
            bail!(
                "\x1eDer Dienst hat den Auftrag angenommen, war aber nach {seconds} Sekunden \
                 noch nicht fertig.\x1fthe service took the job but had not finished after \
                 {seconds} seconds\x1d",
                seconds = PATIENCE.as_secs()
            );
        }

        let frames = http::stream(&asking, &upload.headers, left.min(ONE_ASK))
            .with_context(|| asking.clone())?;

        if let Some(frame) = last_frame(&frames) {
            if let Some(trouble) = went_wrong(&frame) {
                bail!("\x1eDer Auftrag ist fehlgeschlagen\x1fthe job failed\x1d: {trouble}");
            }
            if let Some(named) = json_path(&frame, &where_it_stands) {
                let address = absolute(&named, &upload.endpoint)?;
                permitted(&address, allow_insecure)?;
                return Ok(Finished {
                    address,
                    waited: started.elapsed(),
                });
            }
        }

        // Still working. Never sleep past the deadline, or a job that finishes
        // late is reported as a timeout it did not have.
        std::thread::sleep(BETWEEN_ASKS.min(PATIENCE.saturating_sub(started.elapsed())));
    }
}

/// The path to ask one particular job after.
///
/// Whatever stands in braces is where the id goes, whatever the description
/// calls it — `{jobId}`, `{taskId}`, `{id}`.
///
/// A path and never a whole address: this request carries the favourite's
/// headers, which for a service tool is its key, and resolving a path against
/// the endpoint keeps that key on the host the file was sent to. An address in
/// a poll description would by definition be somewhere else.
fn asking_after(pattern: &str, id: &str) -> Result<String> {
    if !pattern.starts_with('/') {
        bail!(
            "\x1eDer Abholweg muss ein Pfad unter der Adresse des Dienstes sein\
             \x1fthe way back has to be a path under the service's own address\x1d: {pattern}"
        );
    }

    let (before, rest) = pattern.split_once('{').with_context(|| {
        format!(
            "\x1eIm Abholweg steht nicht, wo die Auftragsnummer hingehört\
             \x1fthe way back does not say where the job id goes\x1d: {pattern}"
        )
    })?;
    let (_, after) = rest.split_once('}').with_context(|| {
        format!(
            "\x1eIm Abholweg fehlt die schließende Klammer\
             \x1fthe way back is missing its closing brace\x1d: {pattern}"
        )
    })?;

    Ok(format!("{before}{}{after}", encode(id)))
}

/// Where the finished address stands in a progress frame.
///
/// A frame carries the tool's own answer under `result`, in the very shape the
/// synchronous answer has — so the favourite already says where to look, and a
/// poll description only has to name a place when a service disagrees. A
/// favourite whose result *is* the answer body has nothing to point at, and
/// that is `None`: a job whose result cannot be found is refused rather than
/// waited on forever.
fn frame_path(poll: &Poll, action: &ResultAction) -> Option<String> {
    let named = poll.result.trim();
    if !named.is_empty() {
        return Some(named.to_string());
    }

    match action {
        ResultAction::Save {
            source: ResultSource::Json { path },
            ..
        }
        | ResultAction::Open {
            source: ResultSource::Json { path },
        } => Some(format!("result.{path}")),
        _ => None,
    }
}

/// The last whole frame of a Server-Sent-Events answer.
///
/// A frame is one or more `data:` lines ended by a blank one; several arrive on
/// one connection, and the last is the newest. A trailing frame with no blank
/// line after it is one the connection was cut in the middle of — it is skipped
/// rather than guessed at, and the next ask brings it whole.
fn last_frame(body: &[u8]) -> Option<serde_json::Value> {
    let text = String::from_utf8_lossy(body);
    let mut current = String::new();
    let mut last = None;

    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);

        if line.trim().is_empty() {
            // A frame this cannot read is not a frame: the newest *readable*
            // one stands, rather than nothing at all.
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(current.trim()) {
                last = Some(value);
            }
            current.clear();
            continue;
        }

        if let Some(rest) = line.strip_prefix("data:") {
            // Consecutive data lines are one payload, joined by newlines —
            // which is what the format says, and what a service that sends
            // pretty-printed JSON relies on.
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }

    last
}

/// Whether this frame says the job will never finish.
///
/// Services do not agree on the word or on the field: `phase`, `stage`,
/// `status`, `state`. What they do agree on is that the word contains "fail",
/// "error", "cancel" or "abort" — so that is what is looked for, rather than a
/// list of exact spellings that the next service breaks. Without this a failed
/// job would be waited out for the full two minutes and then reported as slow.
fn went_wrong(frame: &serde_json::Value) -> Option<String> {
    let said = |key: &str| {
        frame
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_ascii_lowercase)
    };

    let bad = ["fail", "error", "cancel", "abort"];
    let broken = ["phase", "stage", "status", "state"]
        .into_iter()
        .filter_map(said)
        .any(|word| bad.iter().any(|mark| word.contains(mark)));

    let why = frame
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());

    match (broken, why) {
        (_, Some(why)) => Some(why.to_string()),
        // An error text of its own is a failure even where no phase says so.
        (true, None) => Some("\x1eohne Begründung\x1fno reason given\x1d".into()),
        (false, None) => None,
    }
}

/// The way back to a job, out of the description of the service the endpoint
/// belongs to.
///
/// For favourites written before any of this existed. Their file says nothing
/// about jobs, but the service they came from is still in `services.json` with
/// the address of its own description — and that description names the path it
/// takes questions on. One extra request, on the click where the answer was a
/// receipt and nowhere else.
///
/// Matched by host rather than by the favourite's id: an id is a naming
/// convention, and a favourite typed by hand follows none.
fn discovered(endpoint: &str, allow_insecure: bool) -> Result<Poll> {
    let host = host_of(endpoint);
    let service = crate::service::load()
        .unwrap_or_default()
        .into_iter()
        .find(|service| host_of(&service.spec_url) == host)
        .with_context(|| {
            format!(
                "\x1eDer Dienst hat den Auftrag eingereiht, und zu {host} steht kein Dienst \
                 in der Liste, der sagen könnte, wo das Ergebnis abzuholen ist.\x1fthe service \
                 queued the job, and no service on the list for {host} can say where to \
                 fetch the result\x1d"
            )
        })?;

    permitted(&service.spec_url, allow_insecure)?;
    let headers: Vec<crate::favourites::Header> = service.auth_header.into_iter().collect();
    let described =
        http::fetch(&service.spec_url, &headers).with_context(|| service.spec_url.clone())?;
    let described: serde_json::Value = serde_json::from_slice(&described).context(
        "\x1eDie Beschreibung des Dienstes ist kein JSON\
         \x1fthe service's description is not JSON\x1d",
    )?;

    let path = crate::service::spec::progress_path(&described);
    if path.trim().is_empty() {
        bail!(
            "\x1eDer Dienst hat den Auftrag eingereiht, seine Beschreibung sagt aber nirgends, \
             wo man nach einem Auftrag fragt.\x1fthe service queued the job, but its \
             description never says where a job is asked after\x1d"
        );
    }
    Ok(Poll::at(path.trim()))
}

/// The half sentence in front of a result that had to be waited for.
///
/// Empty for the ordinary case, so nothing changes for the answer that arrived
/// straight away.
fn took_a_while(job: &Option<Finished>) -> String {
    match job {
        None => String::new(),
        Some(job) => format!(
            "\x1eDer Dienst hat den Auftrag eingereiht und ihn in {seconds:.1} s erledigt.\
             \x1fthe service queued the job and finished it in {seconds:.1} s.\x1d ",
            seconds = job.waited.as_secs_f32()
        ),
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
                bail!("\x1eAntwort ist keine Adresse\x1fthe answer is not an address\x1d")
            }
        }
        ResultSource::Location => answer
            .header("Location")
            .map(str::to_string)
            .context("\x1eKein Location-Kopf in der Antwort\x1fno Location header\x1d"),
        ResultSource::Json { path } => {
            let value: serde_json::Value = serde_json::from_slice(&answer.body)
                .context("\x1eAntwort ist kein JSON\x1fthe answer is not JSON\x1d")?;
            json_path(&value, path).with_context(|| {
                format!("\x1eKein Feld {path} in der Antwort\x1fno field {path} in the answer\x1d")
            })
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

    /// An upload with one endpoint and one thing to do with the answer.
    fn upload(endpoint: &str, result: ResultAction) -> Upload {
        Upload {
            endpoint: endpoint.into(),
            method: "POST".into(),
            body: UploadBody::Multipart {
                field: "file".into(),
            },
            headers: Vec::new(),
            fields: Vec::new(),
            poll: None,
            result,
        }
    }

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
    fn a_queued_job_is_never_saved_as_a_picture() {
        let file = Path::new("bild.png");
        let save = ResultAction::Save {
            source: ResultSource::Body,
            suffix: ".neu".into(),
        };

        // The polite signal. `from: body` means the answer *is* the file, so
        // there is nothing in a progress frame to point at — this must refuse
        // rather than write the receipt out under a picture's name.
        let accepted = http::Answer {
            status: 202,
            headers: Vec::new(),
            body: br#"{"jobId":"abc"}"#.to_vec(),
        };
        assert!(
            apply_result(
                &upload("http://x.invalid/y", save.clone()),
                &accepted,
                file,
                true
            )
            .is_err()
        );

        // And the one that turns up when the description promised 200:
        // measured on a real service, which answers this to a tool its own
        // OpenAPI lists as synchronous.
        let lying = http::Answer {
            status: 200,
            headers: Vec::new(),
            body: br#"{"jobId":"abc","async":true}"#.to_vec(),
        };
        assert!(
            apply_result(
                &upload("http://x.invalid/y", save.clone()),
                &lying,
                file,
                true
            )
            .is_err()
        );

        // Reporting is still allowed: that is the mode for looking at what a
        // service actually says, and it never fetches anything.
        assert!(
            apply_result(
                &upload("http://x.invalid/y", ResultAction::Report),
                &accepted,
                file,
                true
            )
            .is_ok()
        );

        // An ordinary answer is untouched by any of this.
        let ordinary = http::Answer {
            status: 200,
            headers: Vec::new(),
            body: b"nicht wirklich ein PNG".to_vec(),
        };
        let directory = std::env::temp_dir().join("ctxmenu_async_test");
        let _ = std::fs::create_dir_all(&directory);
        let target = directory.join("bild.png");
        assert!(
            apply_result(
                &upload("http://x.invalid/y", save),
                &ordinary,
                &target,
                true
            )
            .is_ok()
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_service_in_the_local_network_is_still_reached_over_http() {
        // The case this program exists for: a tool on the LAN, no certificate,
        // and a user who ticked the box that says so. Every favourite in the
        // author's own list is of this shape, and anything that refuses it has
        // broken the main use rather than secured it.
        let endpoint = "http://192.168.2.11:1349/api/v1/tools/image/compress";
        assert!(permitted(endpoint, true).is_ok());
        assert!(permitted("https://api.tinify.com/shrink", false).is_ok());

        // And the second half of that trip: the service answers with a path,
        // which has to resolve against the endpoint and then be allowed.
        let result = absolute("/api/v1/download/abc/test_compress.png", endpoint)
            .expect("a path resolves against the endpoint it came from");
        assert_eq!(
            result,
            "http://192.168.2.11:1349/api/v1/download/abc/test_compress.png"
        );
        assert!(
            permitted(&result, true).is_ok(),
            "fetching the result from the same host must stay possible"
        );
    }

    #[test]
    fn an_unencrypted_endpoint_is_refused_unless_it_was_allowed() {
        // The tick box used to be read by `problems()` only, which advises and
        // refuses nothing: the file went out in the clear either way.
        let error = permitted("http://tool.example/api/upload", false)
            .expect_err("http:// without the permission");
        let message = format!("{error:#}");
        assert!(
            message.contains("http://") && message.contains("tool.example"),
            "the message has to say which address it means: {message}"
        );
    }

    #[test]
    fn an_address_out_of_an_answer_may_not_be_a_local_program() {
        // `ResultAction::Open` hands the address to ShellExecuteExW with the
        // verb `open`, and for an `.exe` that means run it.
        assert!(permitted("file:///C:/Users/x/Downloads/setup.exe", true).is_err());
        assert!(
            permitted("file:////angreifer/share/payload.exe", true).is_err(),
            "the UNC form is a file: address as well"
        );
        assert!(permitted("javascript:alert(1)", true).is_err());
        assert!(permitted("ftp://example.invalid/x", true).is_err());
    }

    #[test]
    fn a_local_path_in_the_answer_never_reaches_the_shell() {
        // The whole way through, as a favourite would take it: a service that
        // answers with a file: address must not get a program started, and the
        // refusal has to happen before anything is opened.
        let open = ResultAction::Open {
            source: ResultSource::Json { path: "url".into() },
        };
        // A share on a machine that does not exist, deliberately: should this
        // rule ever go missing, the test must not be what starts a program.
        let answer = http::Answer {
            status: 200,
            headers: Vec::new(),
            body: br#"{"url":"file:////angreifer.invalid/share/payload.exe"}"#.to_vec(),
        };
        assert!(
            apply_result(
                &upload("https://tool.example/api", open),
                &answer,
                Path::new("bild.png"),
                true
            )
            .is_err()
        );

        // Same for the Location header, the other way an address arrives.
        let located = http::Answer {
            status: 200,
            headers: vec![(
                "Location".into(),
                "file:////angreifer.invalid/share/payload.exe".into(),
            )],
            body: Vec::new(),
        };
        let open_located = ResultAction::Open {
            source: ResultSource::Location,
        };
        assert!(
            apply_result(
                &upload("https://tool.example/api", open_located),
                &located,
                Path::new("bild.png"),
                true
            )
            .is_err()
        );
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

    /// A server on the loopback interface that answers a fixed sequence of
    /// requests and says what it was asked for.
    ///
    /// The whole way through a queued job cannot be settled without a real
    /// socket: two requests, in order, one of which is a stream. Both cost a
    /// port on 127.0.0.1 and reach no network.
    fn serving(answers: Vec<String>) -> (u16, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let port = listener.local_addr().expect("the port").port();
        let (sender, receiver) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            for answer in answers {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));

                // Up to the empty line: a GET has no body, and waiting for the
                // client to close would mean waiting for the answer first.
                let mut request = Vec::new();
                let mut chunk = [0u8; 4 * 1024];
                while !request.windows(4).any(|four| four == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => request.extend_from_slice(&chunk[..read]),
                    }
                }

                let _ = stream.write_all(answer.as_bytes());
                let _ = stream.flush();
                let _ = sender.send(String::from_utf8_lossy(&request).to_string());
            }
        });

        (port, receiver)
    }

    #[test]
    fn a_queued_job_is_waited_out_and_its_result_lands_on_disk() {
        // Two frames on one stream, the second one terminal: the shape a real
        // service sends, measured on SnapOtter on 2026-08-16.
        let result = b"ein kleines Bild";
        let (port, asked) = serving(vec![
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n\
             data: {\"jobId\":\"abc\",\"phase\":\"processing\",\"percent\":40}\n\n\
             data: {\"jobId\":\"abc\",\"phase\":\"complete\",\"percent\":100,\
             \"result\":{\"downloadUrl\":\"/hole/abc/bild.png\",\"processedSize\":16},\
             \"type\":\"single\"}\n\n"
                .to_string(),
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n{}",
                result.len(),
                String::from_utf8_lossy(result)
            ),
        ]);

        let directory = std::env::temp_dir().join("ctxmenu_job_test");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temp directory");
        let file = directory.join("bild.png");
        std::fs::write(&file, b"das Original").expect("write");

        let mut task = upload(
            &format!("http://127.0.0.1:{port}/api/tools/compress"),
            ResultAction::Save {
                source: ResultSource::Json {
                    path: "downloadUrl".into(),
                },
                suffix: ".neu".into(),
            },
        );
        task.poll = Some(Poll::at("/api/jobs/{jobId}/progress"));

        let receipt = http::Answer {
            status: 202,
            headers: Vec::new(),
            body: br#"{"jobId":"abc","async":true}"#.to_vec(),
        };

        let message = apply_result(&task, &receipt, &file, true).expect("the job is waited out");
        assert!(message.contains("bild.neu.png"), "{message}");
        assert_eq!(
            std::fs::read(directory.join("bild.neu.png")).expect("the result was written"),
            result,
            "the file has to be the one the progress pointed at, not the receipt"
        );
        assert!(
            std::fs::read(&file).expect("the original") == b"das Original",
            "the original is never touched"
        );

        // The two questions, in order and at the addresses the service named.
        let first = asked
            .recv_timeout(Duration::from_secs(5))
            .expect("the job was asked after");
        assert!(
            first.starts_with("GET /api/jobs/abc/progress "),
            "the job id belongs in the path: {first}"
        );
        let second = asked
            .recv_timeout(Duration::from_secs(5))
            .expect("the result was fetched");
        assert!(
            second.starts_with("GET /hole/abc/bild.png "),
            "the address out of the progress frame: {second}"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_job_that_failed_is_reported_instead_of_waited_out() {
        // Without this the two minutes would run out and the answer would be
        // "the service is slow", which is not what happened.
        let (port, _asked) = serving(vec![
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n\
             data: {\"jobId\":\"abc\",\"phase\":\"failed\",\"percent\":100,\
             \"error\":\"unsupported colour space\"}\n\n"
                .to_string(),
        ]);

        let mut task = upload(
            &format!("http://127.0.0.1:{port}/api/tools/compress"),
            ResultAction::Save {
                source: ResultSource::Json {
                    path: "downloadUrl".into(),
                },
                suffix: ".neu".into(),
            },
        );
        task.poll = Some(Poll::at("/api/jobs/{jobId}/progress"));

        let receipt = http::Answer {
            status: 202,
            headers: Vec::new(),
            body: br#"{"jobId":"abc","async":true}"#.to_vec(),
        };

        let started = Instant::now();
        let error = apply_result(&task, &receipt, Path::new("bild.png"), true)
            .expect_err("a failed job is an error");
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "a failed job must not be waited out"
        );
        assert!(
            format!("{error:#}").contains("unsupported colour space"),
            "the service's own reason is the useful half: {error:#}"
        );
    }

    #[test]
    fn the_newest_whole_frame_of_a_stream_is_the_one_that_counts() {
        let stream = b"data: {\"phase\":\"processing\",\"percent\":10}\n\n\
                       : heartbeat\n\n\
                       data: {\"phase\":\"processing\",\"percent\":80}\n\n\
                       data: {\"phase\":\"complete\",\"result\":{\"downloadUrl\":\"/a/b.png\"}}\n\n";
        let frame = last_frame(stream).expect("a whole frame");
        assert_eq!(
            json_path(&frame, "result.downloadUrl").as_deref(),
            Some("/a/b.png")
        );

        // Carriage returns are the other spelling of the same thing.
        let windows = b"data: {\"percent\":5}\r\n\r\ndata: {\"percent\":50}\r\n\r\n";
        assert_eq!(
            json_path(&last_frame(windows).expect("a frame"), "percent").as_deref(),
            Some("50")
        );

        // A frame the connection was cut in the middle of has no blank line
        // after it and is not used: the next ask brings it whole.
        let cut = b"data: {\"percent\":50}\n\ndata: {\"percent\":9";
        assert_eq!(
            json_path(&last_frame(cut).expect("the last whole one"), "percent").as_deref(),
            Some("50")
        );

        // Several data lines are one payload, which is what the format says.
        let split = b"data: {\"percent\":\ndata: 100}\n\n";
        assert_eq!(
            json_path(&last_frame(split).expect("a frame"), "percent").as_deref(),
            Some("100")
        );

        assert!(last_frame(b"").is_none());
        assert!(
            last_frame(b": nur ein Herzschlag\n\n").is_none(),
            "a comment is not a frame"
        );
    }

    #[test]
    fn a_job_that_will_not_finish_is_recognised_whatever_the_service_calls_it() {
        let failed = |json: &str| went_wrong(&serde_json::from_str(json).expect("json"));

        assert_eq!(
            failed(r#"{"phase":"failed","error":"kein Platz mehr"}"#).as_deref(),
            Some("kein Platz mehr")
        );
        // Every field a service might have chosen, and the word inside it
        // rather than a list of exact spellings.
        assert!(failed(r#"{"status":"FAILED"}"#).is_some());
        assert!(failed(r#"{"stage":"error-handling"}"#).is_some());
        assert!(failed(r#"{"state":"cancelled"}"#).is_some());
        // An error text on its own says enough.
        assert!(failed(r#"{"percent":40,"error":"abgebrochen"}"#).is_some());

        // And the ordinary course of a job is not a failure.
        assert!(failed(r#"{"phase":"processing","percent":40}"#).is_none());
        assert!(failed(r#"{"phase":"complete","percent":100}"#).is_none());
        assert!(
            failed(r#"{"phase":"processing","error":""}"#).is_none(),
            "an empty error field is not an error"
        );
    }

    #[test]
    fn the_job_id_goes_where_the_braces_are_and_the_way_back_stays_a_path() {
        assert_eq!(
            asking_after("/api/v1/jobs/{jobId}/progress", "34880766-a0f6").unwrap(),
            "/api/v1/jobs/34880766-a0f6/progress"
        );
        // Whatever the description calls it.
        assert_eq!(
            asking_after("/v2/tasks/{taskId}/status", "abc").unwrap(),
            "/v2/tasks/abc/status"
        );
        // An id out of a foreign answer is encoded like anything else: an
        // answer of `../../etc` must not become a request somewhere else, so
        // its separators do not survive as separators.
        let sneaky = asking_after("/jobs/{id}/progress", "../../etc").unwrap();
        assert_eq!(sneaky, "/jobs/..%2F..%2Fetc/progress");
        assert_eq!(
            sneaky.matches('/').count(),
            3,
            "the path has the three slashes it was written with: {sneaky}"
        );

        // A whole address would take this favourite's key to another host.
        assert!(asking_after("https://woanders.example/jobs/{id}", "abc").is_err());
        assert!(asking_after("jobs/{id}/progress", "abc").is_err());
        // And a way back with nowhere to put the id is no way back.
        assert!(asking_after("/api/v1/jobs/progress", "abc").is_err());
        assert!(asking_after("/api/v1/jobs/{id/progress", "abc").is_err());
    }

    #[test]
    fn a_favourite_that_says_where_its_result_stands_says_it_for_a_job_too() {
        // A progress frame carries the tool's ordinary answer under `result`,
        // so nothing extra has to be configured for the common case.
        let plain = Poll::at("/jobs/{jobId}/progress");
        let save = ResultAction::Save {
            source: ResultSource::Json {
                path: "downloadUrl".into(),
            },
            suffix: ".neu".into(),
        };
        assert_eq!(
            frame_path(&plain, &save).as_deref(),
            Some("result.downloadUrl")
        );
        assert_eq!(
            frame_path(
                &plain,
                &ResultAction::Open {
                    source: ResultSource::Json {
                        path: "output.url".into()
                    }
                }
            )
            .as_deref(),
            Some("result.output.url")
        );

        // A service that puts it somewhere else says so, and that wins.
        let told = Poll {
            result: "output.href".into(),
            ..Poll::at("/jobs/{jobId}/progress")
        };
        assert_eq!(frame_path(&told, &save).as_deref(), Some("output.href"));

        // And a favourite whose result is the answer body has nothing to point
        // at, which is refused rather than guessed at.
        assert_eq!(
            frame_path(
                &plain,
                &ResultAction::Save {
                    source: ResultSource::Body,
                    suffix: ".neu".into()
                }
            ),
            None
        );
        assert_eq!(frame_path(&plain, &ResultAction::Report), None);
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
