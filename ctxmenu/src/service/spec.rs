//! Reads an OpenAPI description and finds the tools that take a file.
//!
//! The point of this module is one question: *which endpoints of this service
//! could sit in a right-click menu?* An endpoint qualifies when it accepts a
//! file as `multipart/form-data`, and everything else about it — what it is
//! called, which group it belongs to, what else it wants, whether it answers
//! straight away — decides whether it can be offered, offered with a form, or
//! only shown with a reason.
//!
//! Deliberately tolerant. A description is written by somebody else's code
//! generator and will contain shapes this does not expect; the answer to that
//! is to skip the endpoint, not to refuse the service.

use std::borrow::Cow;

use serde_json::Value;

/// One endpoint that takes a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    /// `/api/v1/tools/image/compress`, the key exactly as the description
    /// writes it — it is also the anchor its own documentation uses.
    pub path: String,
    /// What the description's `servers` block puts in front of every path: an
    /// address of its own (`https://api.example.com/v2`), a path under the same
    /// host (`/api/v1`), or `/`. Empty when the description says nothing, which
    /// means the same as `/`.
    pub base: String,
    /// Where this service takes questions about a job it has only taken in —
    /// `/api/v1/jobs/{jobId}/progress`. A property of the whole description
    /// rather than of one endpoint, carried here for the same reason as
    /// [`Tool::base`]: this is what a favourite is built from. Empty when the
    /// description offers no such path.
    pub progress: String,
    /// Upper case, as the request will be sent.
    pub method: String,
    /// The group this belongs to, from `tags`. `None` lands under "other".
    pub tag: Option<String>,
    /// Short name for a menu; falls back to the last part of the path.
    pub summary: String,
    /// Longer text, if the description carries one.
    pub description: Option<String>,
    /// The form field the file goes into — usually `file`.
    pub file_field: String,
    /// What else the endpoint wants beside the file.
    pub settings: Settings,
    /// Whether this can be used as it stands.
    pub usable: Usable,
}

/// What an endpoint wants beside the file.
#[derive(Debug, Clone, PartialEq)]
pub enum Settings {
    /// Nothing. The file alone is the whole request.
    None,
    /// One free-text field, with whatever the service said about it.
    ///
    /// The common case in practice: the field is declared `type: string` and
    /// what belongs inside is spelled out in prose. Measured on SnapOtter
    /// (2026-08-15): every one of its 116 image tools describes its options
    /// this way, as Markdown in `description`.
    Text {
        field: String,
        description: Option<String>,
    },
    /// Real fields, from a real schema. A form can be built from these.
    ///
    /// `field` names the one form field they are packed into as JSON. It is
    /// `None` where the service declared each option as a form field of its
    /// own — then every value travels under its own name and there is no such
    /// single place.
    Fields {
        field: Option<String>,
        fields: Vec<Field>,
    },
}

/// One described parameter, enough to draw an input for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub kind: FieldKind,
    pub required: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text,
    Number {
        minimum: Option<f64>,
        maximum: Option<f64>,
    },
    Flag,
    Choice(Vec<String>),
}

/// Whether an endpoint can be turned into a menu entry as it stands.
#[derive(Debug, Clone, PartialEq)]
pub enum Usable {
    /// The file is enough; the answer comes straight back.
    Yes,
    /// Wants something else filled in first.
    NeedsSettings,
    /// Answers `202` with a job id. Fetching the result means asking again
    /// until it is done, which this program cannot do — offering it would
    /// produce an entry that reports success and saves nothing.
    Asynchronous,
}

/// Every file-taking endpoint in this description.
///
/// Returns them in the order the description lists them, which is the order
/// the service's own documentation shows.
pub fn tools(spec: &Value) -> Vec<Tool> {
    let Some(paths) = spec.get("paths").and_then(Value::as_object) else {
        return Vec::new();
    };

    let base = base_url(spec);
    let progress = progress_path(spec);

    let mut out = Vec::new();
    for (path, methods) in paths {
        let Some(methods) = methods.as_object() else {
            continue;
        };
        for (method, operation) in methods {
            // Only the verbs that carry a body. `get` with a file is not a
            // thing, and `delete`/`head` would be a misunderstanding.
            if !matches!(method.as_str(), "post" | "put" | "patch") {
                continue;
            }
            if let Some(mut tool) = read_operation(spec, path, method, operation) {
                tool.base.clone_from(&base);
                tool.progress.clone_from(&progress);
                out.push(tool);
            }
        }
    }
    out
}

/// What the description says its paths hang under.
///
/// The first `servers` entry: the list is written as alternatives, and the
/// first is the one a generator means when it means one. An address with
/// variables in it (`https://{region}.example.com`) is left alone — filling
/// them in would be a guess, and a guessed host answers nothing at all, which
/// is worse than the host the description itself came from.
fn base_url(spec: &Value) -> String {
    spec.get("servers")
        .and_then(Value::as_array)
        .and_then(|list| list.first())
        .and_then(|server| server.get("url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.contains('{'))
        .unwrap_or_default()
        .to_string()
}

fn read_operation(root: &Value, path: &str, method: &str, operation: &Value) -> Option<Tool> {
    let schema = operation
        .get("requestBody")?
        .get("content")?
        .get("multipart/form-data")?
        .get("schema")?;
    let schema = resolve(root, schema, 0);
    // Every property in the shape it really has, so that everything below reads
    // a plain schema whether the description wrote one out or pointed at it.
    let properties: serde_json::Map<String, Value> = schema
        .get("properties")?
        .as_object()?
        .iter()
        .map(|(name, value)| (name.clone(), resolve(root, value, 0).into_owned()))
        .collect();

    // The file field: a string declared as binary. Without one this endpoint
    // is not something a right-click on a file can serve.
    let (file_field, _) = properties.iter().find(|(_, value)| is_file(value))?;

    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let settings = read_settings(root, &properties, file_field, &required);
    // Whether anything has to be filled in first is a question about the whole
    // schema, not about the one field the settings were built from: a required
    // field that no form shows makes a tool that looks ready, sends nothing for
    // it, and is refused on every use.
    let wanted = required.iter().any(|name| {
        *name != file_field.as_str() && !PLUMBING.contains(&name.to_lowercase().as_str())
    });
    let usable = match (answers_asynchronously(operation), wanted) {
        (true, _) => Usable::Asynchronous,
        (_, false) => Usable::Yes,
        (_, true) => Usable::NeedsSettings,
    };

    Some(Tool {
        path: path.to_string(),
        // Both filled in by `tools`, which is where the whole description is in
        // view; one endpoint knows nothing about the servers block, and nothing
        // about the path its service takes questions on.
        base: String::new(),
        progress: String::new(),
        method: method.to_uppercase(),
        tag: operation
            .get("tags")
            .and_then(Value::as_array)
            .and_then(|tags| tags.first())
            .and_then(Value::as_str)
            .map(str::to_string),
        summary: operation
            .get("summary")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| last_segment(path).to_string()),
        description: operation
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        file_field: file_field.clone(),
        settings,
        usable,
    })
}

// ---------------------------------------------------------------------------
// Schemas that stand somewhere else.
// ---------------------------------------------------------------------------

/// How far a chain of `$ref`s is followed.
///
/// A description that points at itself is broken, and the answer to a broken
/// one is to stop reading it, not to recurse until the stack runs out. Eight is
/// past anything a generator writes: FastAPI needs one.
const REF_DEPTH: usize = 8;

/// The schema a value stands for: a `$ref` inside this document followed, an
/// `allOf` merged flat.
///
/// FastAPI — and every other generator that keeps its request bodies under
/// `components/schemas` — writes `{"$ref": "#/components/schemas/Body_..."}`
/// where the schema itself could stand. Read as it is written, such an endpoint
/// has no properties, so no file field, so it never reaches the list: measured
/// against a FastAPI-shaped description, none of its endpoints did, and the
/// service came out empty without saying why.
///
/// Only pointers into this document are followed. One at another file would
/// mean a second request while a panel waits, and the answer to it may be
/// another reference again.
fn resolve<'a>(root: &'a Value, schema: &'a Value, depth: usize) -> Cow<'a, Value> {
    if depth >= REF_DEPTH {
        return Cow::Borrowed(schema);
    }
    if let Some(pointer) = schema.get("$ref").and_then(Value::as_str) {
        return match local(root, pointer) {
            Some(target) => resolve(root, target, depth + 1),
            // A reference that leads nowhere leaves the schema as it stands:
            // the endpoint is skipped, the rest of the description is not.
            None => Cow::Borrowed(schema),
        };
    }
    match schema.get("allOf").and_then(Value::as_array) {
        Some(parts) => Cow::Owned(flattened(root, schema, parts, depth)),
        None => Cow::Borrowed(schema),
    }
}

/// Where a pointer leads inside this document, if it stays inside it.
fn local<'a>(root: &'a Value, pointer: &str) -> Option<&'a Value> {
    let rest = pointer.strip_prefix('#')?;
    match rest.is_empty() {
        true => Some(root),
        false => root.pointer(rest),
    }
}

/// The parts of an `allOf` as the one schema they describe together.
///
/// Properties are collected and `required` lists appended, so that a body split
/// over a shared part and a specific one reads as the whole it stands for.
/// Whatever the schema says beside its `allOf` is written last and wins: it is
/// the more specific word.
fn flattened<'a>(root: &'a Value, schema: &'a Value, parts: &'a [Value], depth: usize) -> Value {
    let mut merged = serde_json::Map::new();
    let mut properties = serde_json::Map::new();
    let mut required: Vec<Value> = Vec::new();

    for part in parts {
        let part = resolve(root, part, depth + 1);
        absorb(&part, &mut merged, &mut properties, &mut required);
    }
    absorb(schema, &mut merged, &mut properties, &mut required);

    if !properties.is_empty() {
        merged.insert("properties".into(), Value::Object(properties));
    }
    if !required.is_empty() {
        merged.insert("required".into(), Value::Array(required));
    }
    Value::Object(merged)
}

/// One part of an `allOf` into the schema being built from it.
fn absorb(
    part: &Value,
    merged: &mut serde_json::Map<String, Value>,
    properties: &mut serde_json::Map<String, Value>,
    required: &mut Vec<Value>,
) {
    let Some(object) = part.as_object() else {
        return;
    };
    for (key, value) in object {
        match key.as_str() {
            // The list itself is what is being read; keeping it would invite a
            // second pass over the same parts.
            "allOf" => {}
            "properties" => {
                if let Some(more) = value.as_object() {
                    for (name, property) in more {
                        properties.insert(name.clone(), property.clone());
                    }
                }
            }
            "required" => {
                if let Some(more) = value.as_array() {
                    required.extend(more.iter().cloned());
                }
            }
            _ => {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Form fields that are plumbing rather than settings.
///
/// Measured on SnapOtter (2026-08-15): every one of its 232 endpoints carries a
/// `clientJobId` beside its real options, for progress reporting this program
/// does not use. Since `serde_json` keeps an object's properties in alphabetical
/// order, `clientJobId` came first and was offered as *the* setting of the tool
/// — with the wrong prose next to it, which is how it was noticed.
const PLUMBING: &[&str] = &[
    "clientjobid",
    "jobid",
    "job_id",
    "requestid",
    "request_id",
    "callbackurl",
    "callback_url",
    "webhook",
    "webhookurl",
    "async",
];

/// Names that mean "this is where the settings go", best first.
const SETTINGS_NAMES: &[&str] = &["settings", "options", "params", "parameters", "config"];

/// What the endpoint wants beside the file, in the most useful form available.
fn read_settings(
    root: &Value,
    properties: &serde_json::Map<String, Value>,
    file_field: &str,
    required: &[&str],
) -> Settings {
    let candidates: Vec<(&String, &Value)> = properties
        .iter()
        .filter(|(name, _)| name.as_str() != file_field)
        // A second file is not a setting. Measured on the test service
        // (2026-08-16): eight endpoints declare one — a mask, a watermark, a
        // signature image, an archive — and this program sends exactly one
        // file. Offered as a box to type in, it asks for an image in words.
        .filter(|(_, value)| !is_file(value))
        .filter(|(name, _)| !PLUMBING.contains(&name.to_lowercase().as_str()))
        .collect();

    // A name the service itself uses for its options, or a described object:
    // either way this one field holds everything, and what goes into it is a
    // JSON block rather than a form field per option.
    let block = SETTINGS_NAMES
        .iter()
        .find_map(|wanted| {
            candidates
                .iter()
                .find(|(name, _)| name.to_lowercase() == *wanted)
        })
        .or_else(|| {
            candidates
                .iter()
                .find(|(_, value)| value.get("properties").is_some())
        });

    if let Some((name, value)) = block {
        return one_field(root, name, value, required);
    }

    match candidates.as_slice() {
        [] => Settings::None,
        // One field that says nothing about its own type: the shape this was
        // built for, where a service names a single place for its options and
        // spells out in prose what belongs inside. One that declares a number,
        // a flag or a list of values is better drawn as what it declares.
        [(name, value)] if kind_of(value) == FieldKind::Text => {
            one_field(root, name, value, required)
        }
        // Several, and none of them the box for all of them: then the service
        // declared each option as a form field of its own. Every one of them
        // has to survive — picking the alphabetically first and dropping the
        // rest sent a request the service refuses, under a tick that says the
        // tool is ready. Measured on the test service (2026-08-16): three
        // endpoints, one of which lost a required field.
        several => Settings::Fields {
            field: None,
            fields: several
                .iter()
                .map(|(name, value)| field_from(name, value, required))
                .collect(),
        },
    }
}

/// The settings of a service that keeps all of them in one form field.
fn one_field(root: &Value, name: &str, value: &Value, required: &[&str]) -> Settings {
    // A described object is the good case: every property becomes an input.
    if let Some(fields) = read_fields(root, value, required) {
        return Settings::Fields {
            field: Some(name.to_string()),
            fields,
        };
    }

    let description = value
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);

    // No schema, but the prose may still list the fields one by one. Measured
    // on the test service: 113 of 227 option descriptions do, and become a form
    // of 431 inputs instead of an empty box asking for JSON.
    if let Some(text) = &description {
        let fields = fields_from_prose(text);
        if !fields.is_empty() {
            return Settings::Fields {
                field: Some(name.to_string()),
                fields,
            };
        }
    }

    Settings::Text {
        field: name.to_string(),
        description,
    }
}

/// The properties of a described object, if this is one.
fn read_fields(root: &Value, value: &Value, outer_required: &[&str]) -> Option<Vec<Field>> {
    let properties = value.get("properties")?.as_object()?;
    let required: Vec<&str> = value
        .get("required")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_else(|| outer_required.to_vec());

    let mut fields = Vec::new();
    for (name, property) in properties {
        let property = resolve(root, property, 0);
        fields.push(field_from(name, &property, &required));
    }
    Some(fields)
}

/// One described property, as the input it asks for.
fn field_from(name: &str, property: &Value, required: &[&str]) -> Field {
    Field {
        name: name.to_string(),
        kind: kind_of(property),
        required: required.contains(&name),
        description: property
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// Which input can hold what a property declares.
fn kind_of(property: &Value) -> FieldKind {
    if let Some(choices) = property.get("enum").and_then(Value::as_array) {
        return FieldKind::Choice(
            choices
                .iter()
                .map(|value| match value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .collect(),
        );
    }
    match type_of(property) {
        Some("integer" | "number") => FieldKind::Number {
            minimum: property.get("minimum").and_then(Value::as_f64),
            maximum: property.get("maximum").and_then(Value::as_f64),
        },
        Some("boolean") => FieldKind::Flag,
        _ => FieldKind::Text,
    }
}

/// The type a property declares, however it is written down.
///
/// OpenAPI 3.1 and JSON Schema 2020-12 spell "or null" as a list of types:
/// `"type": ["integer", "null"]`. Read as a single word — which it is not — a
/// number with a range from 1 to 100 became a free text box, and the range the
/// description had spelled out went unused while the service refused whatever
/// was typed.
fn type_of(property: &Value) -> Option<&str> {
    match property.get("type")? {
        Value::String(name) => Some(name.as_str()),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .find(|name| *name != "null"),
        _ => None,
    }
}

/// A field the file itself goes into: a string declared as binary.
fn is_file(property: &Value) -> bool {
    type_of(property) == Some("string")
        && property.get("format").and_then(Value::as_str) == Some("binary")
}

/// Does this endpoint hand back a job rather than a result?
///
/// `202 Accepted` is the signal, and it is in the description: an endpoint that
/// lists it answers with an id that has to be asked about later.
fn answers_asynchronously(operation: &Value) -> bool {
    operation
        .get("responses")
        .and_then(Value::as_object)
        .is_some_and(|responses| responses.contains_key("202"))
}

/// The path this description offers for asking after a job it only took in.
///
/// Deliberately a rule about shape rather than a name to look for: a `GET`
/// whose path carries a variable — the job id — and whose last segment is
/// `progress` or `status`. On the test service that is
/// `/api/v1/jobs/{jobId}/progress`; on the next one it will be
/// `/v2/tasks/{taskId}/status`, and both fit the same sentence.
///
/// Empty when the description offers nothing of the sort, and empty is an
/// answer: a service that never says where to ask cannot be asked, and a tool
/// of it that queues a job says so instead of guessing at an address.
pub fn progress_path(spec: &Value) -> String {
    let Some(paths) = spec.get("paths").and_then(Value::as_object) else {
        return String::new();
    };

    paths
        .iter()
        .filter(|(path, methods)| {
            path.contains('{') && methods.get("get").is_some_and(|get| !get.is_null())
        })
        .map(|(path, _)| path)
        .find(|path| {
            let tail = last_segment(path).to_ascii_lowercase();
            tail == "progress" || tail == "status"
        })
        .cloned()
        .unwrap_or_default()
}

fn last_segment(path: &str) -> &str {
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

// ---------------------------------------------------------------------------
// Field declarations written as prose.
// ---------------------------------------------------------------------------
//
// A service that declares its options as one `type: string` field usually still
// says what belongs inside — as a bullet list in that field's `description`.
// Measured on the test service (2026-08-16): of its 227 option descriptions,
// 113 become a form of 431 inputs, 105 list nothing and keep their text box,
// and 9 name an array or an object somewhere in the list, which also keeps the
// text box — see below.
//
// The reading is deliberately literal, and every step may fail. A line that
// cannot be read whole is not a field, and a list that contains a declaration
// this cannot draw yields no fields at all, so that the caller keeps its
// free-text box. A wrongly read field would be sent to a real service under a
// name or a type it does not know; a missed one costs a checkbox.

/// Reads field declarations out of the prose a service wrote about its options.
///
/// The shape understood is the Markdown bullet list that Zod and JSON-Schema
/// documentation generators emit:
///
/// ```text
/// JSON string with options:
/// - `left` (number, required) - Left offset in pixels (min 0)
/// - `unit` (string, optional) - One of: px, percent
/// ```
///
/// Variants of it are accepted: `*`, `+`, `•`, a dash of any width or `1.` as
/// the bullet; `**name**`, `*name*`, `__name__` or a bare name instead of
/// backticks; `:`, `--` or an em dash instead of ` - `; a bracket that names
/// only `optional`; no bracket at all when the sentence names a default or a
/// list of values.
///
/// Nothing is guessed. A line becomes a field only when something in it
/// *declares* — a type word, `required`/`optional`, a default with a value, or
/// a `One of:` list. That is what separates a declaration from the glossary
/// lists documentation is full of, where ``- `png` - Lossless, supports
/// transparency`` names a value rather than a setting.
///
/// Two shapes end the reading of a whole description, because a form built
/// from what is left would be worse than no form: a declaration whose name is
/// not a key that can be filled in (`steps[].toolId`), and one whose type no
/// single input can hold (`object`, `array`, `string[]`). Both return an empty
/// list, and the caller falls back to the free-text field it had before —
/// where the user can still type the array by hand.
pub fn fields_from_prose(text: &str) -> Vec<Field> {
    let mut fields: Vec<Field> = Vec::new();
    // The indent of the list currently being read. Bullets deeper than this
    // belong to the item above them and are that item's business, not the
    // settings object's. Taking the first bullet of each list rather than the
    // smallest indent in the whole text keeps a note further down from
    // redefining what "outermost" means.
    let mut base: Option<usize> = None;

    for line in text.lines() {
        let Some((indent, body)) = split_bullet(line) else {
            // A paragraph of its own ends the list; the next one starts fresh.
            if !line.trim().is_empty() && indent_of(line) <= base.unwrap_or(0) {
                base = None;
            }
            continue;
        };
        let outermost = *base.get_or_insert(indent);
        if indent > outermost {
            continue;
        }
        match read_declaration(body) {
            Reading::Prose => {}
            Reading::Undrawable => return Vec::new(),
            Reading::Declared(field) => {
                if !fields.iter().any(|seen| seen.name == field.name) {
                    fields.push(field);
                }
            }
        }
    }
    fields
}

/// What one list item turned out to be.
enum Reading {
    /// A field, ready to be drawn.
    Declared(Field),
    /// Not a declaration: a note, a value, a sentence.
    Prose,
    /// A declaration, but not one an input can hold. The whole list has to be
    /// given up, or the form would quietly drop a setting the user needs.
    Undrawable,
}

// --- the line -----------------------------------------------------------

/// How far a line is indented, counting a tab as four columns.
fn indent_of(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

/// The indent and the text of a list item, if the line is one.
///
/// A marker has to be followed by a space, which is what tells `- text` from a
/// `---` rule and `**bold**` from a `*` bullet.
fn split_bullet(line: &str) -> Option<(usize, &str)> {
    let rest = line.trim_start();
    let body = strip_marker(rest)?;
    if !body.starts_with(char::is_whitespace) {
        return None;
    }
    Some((indent_of(line), body.trim()))
}

/// What follows the bullet marker, if the text opens with one.
fn strip_marker(rest: &str) -> Option<&str> {
    let mut chars = rest.chars();
    let first = chars.next()?;
    if matches!(
        first,
        '-' | '*' | '+' | '\u{2022}' | '\u{00B7}' | '\u{2013}' | '\u{2014}'
    ) {
        return Some(chars.as_str());
    }
    // `1.` and `2)` are bullets as well, and generators do write them.
    if first.is_ascii_digit() {
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let after = &rest[digits..];
        if let Some(body) = after.strip_prefix('.').or_else(|| after.strip_prefix(')')) {
            return Some(body);
        }
    }
    None
}

/// One list item, read as a field declaration — or not.
fn read_declaration(body: &str) -> Reading {
    let Some(read) = read_name(body) else {
        return Reading::Prose;
    };
    let Name {
        text: name,
        marked,
        required_marker,
        optional_marker,
        rest,
    } = read;

    let (bracket, rest) = take_bracket(rest);
    let Some(tail) = strip_separator(rest) else {
        // Text that follows the name without a separator is a sentence about
        // the name, not a description of it: "- Quality (integer) defaults to
        // 82 for JPEG output" declares nothing.
        return Reading::Prose;
    };

    let mut declared = Declared::default();
    if let Some(bracket) = bracket {
        declared.read_bracket(bracket);
    }
    let choices = choices_in(tail);
    if !declared.declares() {
        // No bracket, or nothing in it that declares. The sentence may still
        // do it — but only with a spelled-out default or a list of values.
        declared.default = default_in(tail);
        if declared.default.is_none() && choices.is_none() {
            return Reading::Prose;
        }
        // On this thin evidence the name has to look like a key rather than
        // like the start of a sentence, or every "- **Note** - Default: 80 is
        // used for JPEG" turns into a field named `Note`.
        if !starts_lower(name) {
            return Reading::Prose;
        }
    }
    if !marked && !starts_lower(name) {
        // An unmarked capitalised word is the first word of a sentence far
        // more often than it is a key: "- Width (number) - is capped".
        return Reading::Prose;
    }
    if !is_key(name) {
        return Reading::Undrawable;
    }

    match declared.kind(tail, choices) {
        Some(kind) => Reading::Declared(Field {
            name: name.to_string(),
            kind,
            required: (declared.required == Some(true) || required_marker) && !optional_marker,
            description: (!tail.is_empty()).then(|| tail.to_string()),
        }),
        None => Reading::Undrawable,
    }
}

/// The name at the head of a list item.
struct Name<'a> {
    text: &'a str,
    /// Was it marked up as a name — backticks, bold, italics, quotes?
    marked: bool,
    /// A `*` or `!` stuck to the name, which some generators use for "required".
    required_marker: bool,
    /// A `?` stuck to the name, which is how the other half write "optional".
    optional_marker: bool,
    rest: &'a str,
}

/// Wrappers a name may come in, longest first so `**` is not read as an empty
/// `*` pair.
const WRAPPERS: [&str; 6] = ["**", "__", "`", "*", "_", "\""];

fn read_name(body: &str) -> Option<Name<'_>> {
    let mut marked = false;
    let mut text = body;
    let mut rest = "";

    for wrapper in WRAPPERS {
        if let Some(after) = body.strip_prefix(wrapper)
            && let Some(end) = after.find(wrapper)
        {
            marked = true;
            text = &after[..end];
            rest = &after[end + wrapper.len()..];
            break;
        }
    }
    if marked {
        // `**`name`**`: keep peeling as long as both ends match.
        loop {
            let inner = text.trim();
            let Some(peeled) = WRAPPERS.iter().find_map(|wrapper| {
                inner
                    .strip_prefix(wrapper)
                    .and_then(|rest| rest.strip_suffix(wrapper))
            }) else {
                break;
            };
            text = peeled;
        }
    } else {
        let end = body.find(|c: char| !is_key_char(c)).unwrap_or(body.len());
        if end == 0 {
            return None;
        }
        text = &body[..end];
        rest = &body[end..];
    }

    let text = text.trim();
    // `width*` and `width!` mean required; `width?` means optional. A `*`
    // behind the markup counts too, but only when a space follows it — before
    // a bracket it is the italics of `*(number, optional)*`.
    let (text, mut required_marker) = match text.strip_suffix(['*', '!']) {
        Some(shortened) => (shortened.trim_end(), true),
        None => (text, false),
    };
    let (text, optional_marker) = match text.strip_suffix('?') {
        Some(shortened) => (shortened, true),
        None => (text, false),
    };
    if let Some(after) = rest.strip_prefix('*')
        && after.starts_with(char::is_whitespace)
    {
        required_marker = true;
        rest = after;
    }

    Some(Name {
        text,
        marked,
        required_marker,
        optional_marker,
        rest,
    })
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '$')
}

/// Could this be a key of a settings object?
///
/// Deliberately narrow. `steps[].toolId` and `width or height` are described
/// in the very same lists and are not keys anybody can fill in.
fn is_key(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(is_key_char)
}

fn starts_lower(name: &str) -> bool {
    name.starts_with(|c: char| c.is_lowercase() || c == '_')
}

/// The bracket that belongs to the name, and what follows it.
///
/// Only a bracket that stands directly behind the name is the type
/// specification; one further along belongs to the sentence. Italics around
/// it — `*(number, optional)*` — are stripped, that is how several generators
/// write it.
fn take_bracket(rest: &str) -> (Option<&str>, &str) {
    let trimmed = rest.trim_start();
    let opened = trimmed.trim_start_matches(['*', '_']);
    let Some(after) = opened.strip_prefix('(') else {
        return (None, rest);
    };

    let mut depth = 1usize;
    let mut quote: Option<char> = None;
    for (at, c) in after.char_indices() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => {
                depth -= 1;
                if depth == 0 {
                    let tail = after[at + 1..].trim_start_matches(['*', '_']);
                    return (Some(&after[..at]), tail);
                }
            }
            (None, _) => {}
        }
    }
    // A bracket that never closes: leave the line as it was, and the missing
    // separator will throw it away.
    (None, rest)
}

/// What the line says about the field, after the separator that announces it.
///
/// Nothing left is fine — the declaration ends with its bracket. Text without
/// a separator in front of it is not.
fn strip_separator(rest: &str) -> Option<&str> {
    let rest = rest.trim_start();
    if rest.is_empty() {
        return Some("");
    }
    if let Some(after) = rest.strip_prefix([':', '\u{2192}']) {
        return Some(after.trim());
    }
    for dash in ["--", "-", "\u{2014}", "\u{2013}"] {
        if let Some(after) = rest.strip_prefix(dash) {
            if after.is_empty() {
                return Some("");
            }
            if after.starts_with(char::is_whitespace) {
                return Some(after.trim());
            }
        }
    }
    None
}

// --- the bracket --------------------------------------------------------

/// What the bracket behind a name declared.
#[derive(Default)]
struct Declared {
    base: Option<Base>,
    required: Option<bool>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    /// A default was named, and what its value looked like.
    default: Option<Base>,
    /// Something in the bracket was understood at all.
    understood: bool,
}

/// The families of input that can be drawn. Everything else is prose.
#[derive(Clone, Copy, PartialEq)]
enum Base {
    Textual,
    Numeric,
    Boolean,
    /// An object, an array, a `string[]`: real, but nothing a single input can
    /// hold.
    Container,
}

impl Declared {
    /// Did anything here declare a field, rather than describe one?
    fn declares(&self) -> bool {
        self.understood
    }

    fn read_bracket(&mut self, bracket: &str) {
        let mut loose_range = None;
        for part in split_commas(bracket) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let lower = part.to_lowercase();
            match lower.as_str() {
                "required" | "mandatory" | "erforderlich" | "pflicht" => {
                    self.required = Some(true);
                    self.understood = true;
                }
                "optional" | "nullable" | "not required" => {
                    self.required = self.required.or(Some(false));
                    self.understood = true;
                }
                "deprecated" | "readonly" | "read-only" => self.understood = true,
                _ => {
                    if let Some(value) = after_word(&lower, &["default", "defaults to", "standard"])
                    {
                        self.default = Some(shape_of(value));
                        self.required = self.required.or(Some(false));
                        self.understood = true;
                    } else if let Some(value) = after_word(&lower, &["min", "minimum", "at least"])
                    {
                        self.minimum = number(value);
                        self.understood = true;
                    } else if let Some(value) = after_word(&lower, &["max", "maximum", "at most"]) {
                        self.maximum = number(value);
                        self.understood = true;
                    } else if let Some((base, bounds)) = read_type(&lower) {
                        // A type counts once. Further on it is a remark, as in
                        // "(string, as a hex colour)".
                        if self.base.is_none() {
                            self.base = Some(base);
                            self.minimum = self.minimum.or(bounds.0);
                            self.maximum = self.maximum.or(bounds.1);
                            self.understood = true;
                        }
                    } else if let Some(bounds) = read_range(&lower) {
                        // "(number, 0-100)": the range in a part of its own.
                        loose_range = Some(bounds);
                    }
                }
            }
        }
        // A range means numbers — but only where the type does not say
        // otherwise. On "(string, 1-256)" it counts characters, and a spinner
        // would then cap the text at 256 instead of allowing 256 of them. A
        // bracket that holds nothing but a range says number by itself.
        if let Some((low, high)) = loose_range
            && matches!(self.base, None | Some(Base::Numeric))
        {
            self.base = Some(Base::Numeric);
            self.understood = true;
            self.minimum = self.minimum.or(low);
            self.maximum = self.maximum.or(high);
        }
    }

    /// What kind of input this deserves, or nothing when no input can hold it.
    fn kind(&self, tail: &str, choices: Option<Vec<String>>) -> Option<FieldKind> {
        let base = match self.base {
            Some(base) => base,
            // No type word. A default that is plainly a number or a boolean
            // still says what the value is.
            None => match self.default {
                Some(Base::Numeric) => Base::Numeric,
                Some(Base::Boolean) => Base::Boolean,
                _ => Base::Textual,
            },
        };
        match base {
            Base::Container => None,
            Base::Boolean => Some(FieldKind::Flag),
            Base::Numeric => {
                let (low, high) = bound_behind(tail);
                Some(ordered(self.minimum.or(low), self.maximum.or(high)))
            }
            // A list of values only becomes a drop-down for a textual field.
            // On a number the entries would travel as strings, and a service
            // that asked for 90 rejects "90".
            Base::Textual => Some(choices.map_or(FieldKind::Text, FieldKind::Choice)),
        }
    }
}

/// A number field, with a range only where the two ends make one.
///
/// "(number, min 5, max 1)" is a misread somewhere, and a spinner that accepts
/// nothing is worse than one without limits.
fn ordered(minimum: Option<f64>, maximum: Option<f64>) -> FieldKind {
    match (minimum, maximum) {
        (Some(low), Some(high)) if low > high => FieldKind::Number {
            minimum: None,
            maximum: None,
        },
        _ => FieldKind::Number { minimum, maximum },
    }
}

/// `string`, `hex string`, `integer >= 0`, `number 0-100`, `array of {id}`.
///
/// Only the first and the last word of the leading run of words are read as
/// the type; the ones between them qualify it. That is what keeps the English
/// language out: "(Mean Time to Detect)" has `time` in the middle of it and
/// declares nothing, while "hex string" and "number of pages" still do.
///
/// Written as a loop rather than as recursion: a bracket of ten thousand words
/// is a strange thing to meet, but not a reason to run out of stack.
fn read_type(lower: &str) -> Option<(Base, Bounds)> {
    let mut words: Vec<&str> = Vec::new();
    let mut rest = lower.trim();
    while !rest.is_empty() {
        let (word, after) = match rest.find(char::is_whitespace) {
            Some(at) => (&rest[..at], rest[at..].trim_start()),
            None => (rest, ""),
        };
        let bare = word.trim_end_matches("[]");
        if bare.len() < word.len() {
            // `string[]`, `number[]`: an array however it is written.
            return Some((Base::Container, (None, None)));
        }
        // A word is a word while it is made of letters; "0-100" and ">=" are
        // where the type ends and the range begins.
        if !bare.chars().all(|c| c.is_alphabetic() || c == '/') {
            break;
        }
        words.push(bare);
        rest = after;
    }

    if words
        .iter()
        .any(|word| base_of(word) == Some(Base::Container))
    {
        return Some((Base::Container, (None, None)));
    }
    let base = base_of(words.last()?).or_else(|| base_of(words.first()?))?;
    let bounds = if base == Base::Numeric {
        read_range(rest).unwrap_or((None, None))
    } else {
        (None, None)
    };
    Some((base, bounds))
}

/// The type words. Kept short on purpose: every word here is a word that turns
/// a bracket into a declaration, so `time`, `id` or `path` — which stand in
/// English sentences far more often than in type specifications — are not on
/// the list.
fn base_of(word: &str) -> Option<Base> {
    match word {
        "boolean" | "bool" | "flag" | "switch" | "checkbox" | "true/false" | "yes/no"
        | "wahrheitswert" => Some(Base::Boolean),
        "number" | "integer" | "int" | "float" | "double" | "decimal" | "numeric" | "zahl"
        | "ganzzahl" | "kommazahl" => Some(Base::Numeric),
        "string" | "str" | "enum" | "char" | "chars" | "hex" | "color" | "colour" | "date"
        | "datetime" | "zeichenkette" => Some(Base::Textual),
        "object" | "array" | "list" | "map" | "dict" | "dictionary" | "record" | "json"
        | "tuple" | "objekt" | "liste" => Some(Base::Container),
        _ => None,
    }
}

/// A lower and an upper bound, either of which the prose may leave out.
type Bounds = (Option<f64>, Option<f64>);

/// `0-100`, `-100 to 100`, `>= 0`, `<= 5`, `0.25-4`.
///
/// Everything has to be accounted for. A range that cannot be read whole is no
/// range, because half of one would silently bar valid values.
fn read_range(text: &str) -> Option<Bounds> {
    let text = text.trim();
    for (mark, is_minimum) in [
        (">=", true),
        ("\u{2265}", true),
        (">", true),
        ("<=", false),
        ("\u{2264}", false),
        ("<", false),
    ] {
        if let Some(rest) = text.strip_prefix(mark) {
            let bound = number(rest)?;
            return Some(if is_minimum {
                (Some(bound), None)
            } else {
                (None, Some(bound))
            });
        }
    }
    for split in [" to ", "\u{2026}", "..", "\u{2013}"] {
        if let Some((low, high)) = text.split_once(split) {
            return Some((Some(number(low)?), Some(number(high)?)));
        }
    }
    // `0-100`, `0.25-4`, `-80--20`: the separating hyphen is the one that
    // follows a digit, so a leading minus sign is not mistaken for it.
    let bytes = text.as_bytes();
    let at = (1..bytes.len()).find(|&at| bytes[at] == b'-' && bytes[at - 1].is_ascii_digit())?;
    Some((Some(number(&text[..at])?), Some(number(&text[at + 1..])?)))
}

/// A plain decimal number. `inf` and `NaN` parse as floats and are not one.
fn number(text: &str) -> Option<f64> {
    let text = text.trim().trim_end_matches(['.', ',', ';']).trim();
    if !text.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '-' | '+' | '.')) {
        return None;
    }
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// Is this literal a number, a boolean, or something textual?
fn shape_of(value: &str) -> Base {
    let value = value.trim().trim_matches(['"', '\'', '`']);
    match value {
        "true" | "false" => Base::Boolean,
        _ if number(value).is_some() => Base::Numeric,
        _ => Base::Textual,
    }
}

/// What follows one of these words, when the text opens with it.
///
/// The word has to be followed by a separator and by something: `min 0` and
/// `min: 0` name a bound, `mint green` does not.
fn after_word<'a>(text: &'a str, words: &[&str]) -> Option<&'a str> {
    for word in words {
        let Some(rest) = text.strip_prefix(word) else {
            continue;
        };
        if !rest.starts_with([' ', '\t', ':', '=']) {
            continue;
        }
        let rest = rest.trim_start_matches([' ', '\t', ':', '=']).trim();
        if !rest.is_empty() {
            return Some(rest);
        }
    }
    None
}

/// Splits on commas that are outside brackets and quotes.
fn split_commas(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut start = 0usize;
    for (at, c) in text.char_indices() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')' | ']' | '}') => depth = depth.saturating_sub(1),
            (None, ',') if depth == 0 => {
                parts.push(&text[start..at]);
                start = at + 1;
            }
            (None, _) => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

// --- the sentence behind the field --------------------------------------

/// `Default: 80` written into the sentence rather than into the bracket.
///
/// The colon is required. "the default tier" and "defaults to whatever the
/// image has" are prose about a value, not the declaration of one.
fn default_in(tail: &str) -> Option<Base> {
    let after = ["defaults", "default", "standard", "vorgabe"]
        .iter()
        .find_map(|word| find_word(tail, word).map(|at| &tail[at + word.len()..]))?;
    let value = after.trim_start().strip_prefix([':', '='])?.trim();
    let value = value.split_whitespace().next()?;
    (!value.is_empty()).then(|| shape_of(value))
}

/// A trailing bracket that holds a bound and nothing else: `(min 0)`, `(1-100)`.
///
/// Only the last bracket of the sentence counts, and only when the sentence
/// ends with it, so that "(0 = no limit)" and "(must be after `start`)" stay
/// what they are: remarks.
fn bound_behind(tail: &str) -> Bounds {
    let trimmed = tail.trim_end_matches(['.', ' ']);
    let Some(inside) = trimmed.strip_suffix(')') else {
        return (None, None);
    };
    let Some(open) = inside.rfind('(') else {
        return (None, None);
    };
    let inside = inside[open + 1..].to_lowercase();
    if let Some(value) = after_word(&inside, &["min", "minimum", "at least"]) {
        return (number(value), None);
    }
    if let Some(value) = after_word(&inside, &["max", "maximum", "at most"]) {
        return (None, number(value));
    }
    read_range(&inside).unwrap_or((None, None))
}

/// Phrases that announce a list of allowed values.
const CHOICE_PHRASES: [&str; 5] = ["one of", "any of", "either of", "eines von", "einer von"];

/// The values behind a `One of: …`, when every one of them is a value.
///
/// All or nothing: a list with one entry that cannot be read is dropped whole,
/// because a drop-down missing an option cannot say what the user means.
fn choices_in(tail: &str) -> Option<Vec<String>> {
    let at = CHOICE_PHRASES
        .iter()
        .find_map(|phrase| clause_with(tail, phrase))?;
    let list = tail[at..].trim_start_matches([':', ' ', '\t']);

    // The list ends where the sentence does. `1.5` keeps its dot, because the
    // one that ends a sentence is followed by a space or by nothing.
    let end = list
        .char_indices()
        .find(|&(at, c)| {
            matches!(c, '.' | ';')
                && list[at + c.len_utf8()..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })
        .map_or(list.len(), |(at, _)| at);

    let mut values: Vec<String> = Vec::new();
    for piece in list[..end]
        .split([',', '|'])
        .flat_map(|piece| piece.split(" or ").flat_map(|piece| piece.split(" oder ")))
    {
        let piece = piece.trim();
        let piece = piece
            .strip_prefix("or ")
            .or_else(|| piece.strip_prefix("and "))
            .or_else(|| piece.strip_prefix("oder "))
            .unwrap_or(piece);
        // "attention (alias for subject)" names the value and remarks on it.
        let piece = match piece.find('(') {
            Some(at) => &piece[..at],
            None => piece,
        };
        let value = piece.trim().trim_matches(['`', '"', '\'']).trim();
        if !is_value(value) {
            return None;
        }
        if !values.iter().any(|seen| seen == value) {
            values.push(value.to_string());
        }
    }
    (values.len() >= 2).then_some(values)
}

/// Where a phrase begins a clause of its own, rather than sitting inside one.
///
/// Returns the offset just behind it. "Unit, one of: px, percent" announces a
/// list; "Mutually exclusive with one of `preset`, `tier`" mentions other
/// fields, and reading that as a drop-down would offer their names as values.
fn clause_with(tail: &str, phrase: &str) -> Option<usize> {
    let mut from = 0usize;
    while let Some(at) = find_word(&tail[from..], phrase) {
        let at = from + at;
        let before = tail[..at].trim_end().chars().next_back();
        if before.is_none_or(|c| matches!(c, '.' | ',' | ';' | ':' | '(' | '-' | '\u{2014}')) {
            return Some(at + phrase.len());
        }
        from = at + phrase.len();
    }
    None
}

/// Where an ASCII phrase stands, whatever case it is written in, as a word of
/// its own.
///
/// Searching a lowercased copy and indexing the original would be wrong:
/// lowercasing can change a string's length, and the two would drift apart
/// until a slice landed inside a character.
fn find_word(haystack: &str, needle: &str) -> Option<usize> {
    let (bytes, wanted) = (haystack.as_bytes(), needle.as_bytes());
    (0..bytes.len().checked_sub(wanted.len())? + 1).find(|&at| {
        haystack.is_char_boundary(at)
            && bytes[at..at + wanted.len()].eq_ignore_ascii_case(wanted)
            && !bytes[..at]
                .last()
                .is_some_and(|c| c.is_ascii_alphanumeric())
            && haystack[at + wanted.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric())
    })
}

/// Could this be a value the service accepts literally?
fn is_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 40
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:+/@#%".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A description shaped like the ones this was measured against.
    fn spec() -> Value {
        serde_json::json!({
            "paths": {
                "/tools/image/compress": {
                    "post": {
                        "tags": ["Tools"],
                        "summary": "Compress Image",
                        "requestBody": { "content": { "multipart/form-data": { "schema": {
                            "type": "object",
                            "properties": {
                                "file": { "type": "string", "format": "binary" },
                                "settings": { "type": "string", "description": "JSON string with options" }
                            },
                            "required": ["file"]
                        }}}},
                        "responses": { "200": { "description": "Processed image" } }
                    }
                },
                "/tools/image/convert": {
                    "post": {
                        "tags": ["Tools"],
                        "summary": "Convert",
                        "requestBody": { "content": { "multipart/form-data": { "schema": {
                            "properties": {
                                "file": { "type": "string", "format": "binary" },
                                "options": { "type": "object", "properties": {
                                    "format": { "type": "string", "enum": ["png", "webp"], "description": "Output" },
                                    "quality": { "type": "integer", "minimum": 1, "maximum": 100 },
                                    "strip": { "type": "boolean" }
                                }, "required": ["format"] }
                            },
                            "required": ["file", "options"]
                        }}}},
                        "responses": { "200": {} }
                    }
                },
                "/tools/image/sharpen": {
                    "post": {
                        "tags": ["Tools"],
                        "requestBody": { "content": { "multipart/form-data": { "schema": {
                            "properties": { "file": { "type": "string", "format": "binary" } }
                        }}}},
                        "responses": { "202": { "description": "Accepted" } }
                    }
                },
                "/auth/login": {
                    "post": {
                        "requestBody": { "content": { "application/json": { "schema": {
                            "properties": { "username": { "type": "string" } }
                        }}}},
                        "responses": { "200": {} }
                    }
                },
                "/tools/image/list": { "get": { "responses": { "200": {} } } }
            }
        })
    }

    #[test]
    fn the_way_back_to_a_job_is_recognised_by_its_shape_not_by_its_name() {
        // What the test service writes, and the shape the rule is about: a GET,
        // a variable in the path, and the word at the end.
        let snapotter = serde_json::json!({ "paths": {
            "/api/v1/tools/image/compress": { "post": { "responses": { "200": {} } } },
            "/api/v1/jobs/{jobId}/progress": { "get": { "summary": "Job progress (SSE)" } }
        }});
        assert_eq!(progress_path(&snapotter), "/api/v1/jobs/{jobId}/progress");

        // A different service, the same sentence.
        let other = serde_json::json!({ "paths": {
            "/v2/tasks/{taskId}/status": { "get": { "responses": { "200": {} } } }
        }});
        assert_eq!(progress_path(&other), "/v2/tasks/{taskId}/status");

        // Nothing of the sort is an answer, not a reason to guess: this
        // description says nowhere to ask, so nobody is asked.
        assert_eq!(progress_path(&spec()), "");
        assert_eq!(progress_path(&serde_json::json!({})), "");
        // A progress path without a job in it addresses no particular job, and
        // one that is not fetched with GET is not a question.
        assert_eq!(
            progress_path(&serde_json::json!({ "paths": {
                "/jobs/progress": { "get": {} },
                "/jobs/{jobId}/progress": { "post": {} }
            }})),
            ""
        );

        // And it travels with every tool of that description, the way the
        // servers block does -- that is what a favourite is built from.
        let carried = tools(&serde_json::json!({ "paths": {
            "/tools/compress": { "post": {
                "requestBody": { "content": { "multipart/form-data": { "schema": {
                    "properties": { "file": { "type": "string", "format": "binary" } }
                }}}},
                "responses": { "200": {} }
            }},
            "/jobs/{jobId}/progress": { "get": {} }
        }}));
        assert_eq!(carried.len(), 1);
        assert_eq!(carried[0].progress, "/jobs/{jobId}/progress");
    }

    #[test]
    fn only_endpoints_that_take_a_file_are_offered() {
        let found = tools(&spec());
        let paths: Vec<&str> = found.iter().map(|t| t.path.as_str()).collect();

        assert!(paths.contains(&"/tools/image/compress"));
        assert!(paths.contains(&"/tools/image/convert"));
        assert!(paths.contains(&"/tools/image/sharpen"));
        // A login takes JSON, and a listing takes nothing at all. Neither is
        // something a right-click on a file could serve.
        assert!(!paths.contains(&"/auth/login"));
        assert!(!paths.contains(&"/tools/image/list"));
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn a_file_and_an_optional_setting_is_ready_to_use() {
        let found = tools(&spec());
        let compress = found.iter().find(|t| t.path.ends_with("compress")).unwrap();

        assert_eq!(compress.usable, Usable::Yes, "the file alone is enough");
        assert_eq!(compress.file_field, "file");
        assert_eq!(compress.method, "POST");
        assert_eq!(compress.tag.as_deref(), Some("Tools"));
        assert_eq!(compress.summary, "Compress Image");
        // Not a described object, so the best on offer is a text field with
        // whatever the service said about it.
        assert_eq!(
            compress.settings,
            Settings::Text {
                field: "settings".into(),
                description: Some("JSON string with options".into())
            }
        );
    }

    #[test]
    fn a_progress_field_is_not_mistaken_for_the_settings_of_a_tool() {
        // Exactly the shape SnapOtter sends, alphabetical order and all: the
        // plumbing field sorts before the real one, and taking the first field
        // that was not the file put the wrong prose on screen.
        let spec = serde_json::json!({
            "paths": {
                "/api/v1/tools/audio/aac-to-mp3": {
                    "post": {
                        "tags": ["Tools"],
                        "summary": "AAC to MP3",
                        "requestBody": { "content": { "multipart/form-data": { "schema": {
                            "type": "object",
                            "required": ["file"],
                            "properties": {
                                "file": { "type": "string", "format": "binary" },
                                "clientJobId": {
                                    "type": "string",
                                    "description": "Client-provided job ID for SSE progress tracking"
                                },
                                "settings": {
                                    "type": "string",
                                    "description": "Dedicated AAC to MP3 converter."
                                }
                            }
                        }}}},
                        "responses": { "200": { "description": "ok" } }
                    }
                }
            }
        });

        let found = tools(&spec);
        assert_eq!(
            found[0].settings,
            Settings::Text {
                field: "settings".into(),
                description: Some("Dedicated AAC to MP3 converter.".into())
            }
        );
    }

    #[test]
    fn an_endpoint_whose_only_extra_field_is_plumbing_wants_nothing() {
        let spec = serde_json::json!({
            "paths": { "/x": { "post": {
                "summary": "X",
                "requestBody": { "content": { "multipart/form-data": { "schema": {
                    "type": "object",
                    "required": ["file"],
                    "properties": {
                        "file": { "type": "string", "format": "binary" },
                        "clientJobId": { "type": "string" }
                    }
                }}}},
                "responses": { "200": { "description": "ok" } }
            }}}
        });

        let found = tools(&spec);
        assert_eq!(found[0].settings, Settings::None);
        assert_eq!(found[0].usable, Usable::Yes);
    }

    #[test]
    fn a_described_object_becomes_fields_a_form_can_draw() {
        let found = tools(&spec());
        let convert = found.iter().find(|t| t.path.ends_with("convert")).unwrap();

        assert_eq!(
            convert.usable,
            Usable::NeedsSettings,
            "its options are required"
        );
        let Settings::Fields { field, fields } = &convert.settings else {
            panic!("expected described fields, got {:?}", convert.settings);
        };
        assert_eq!(field.as_deref(), Some("options"));

        let format = fields.iter().find(|f| f.name == "format").unwrap();
        assert_eq!(
            format.kind,
            FieldKind::Choice(vec!["png".into(), "webp".into()])
        );
        assert!(format.required);

        let quality = fields.iter().find(|f| f.name == "quality").unwrap();
        assert_eq!(
            quality.kind,
            FieldKind::Number {
                minimum: Some(1.0),
                maximum: Some(100.0)
            }
        );
        assert!(!quality.required, "not in the object's required list");

        let strip = fields.iter().find(|f| f.name == "strip").unwrap();
        assert_eq!(strip.kind, FieldKind::Flag);
    }

    #[test]
    fn an_endpoint_that_answers_with_a_job_is_marked_rather_than_offered() {
        let found = tools(&spec());
        let sharpen = found.iter().find(|t| t.path.ends_with("sharpen")).unwrap();

        // It takes a file and needs nothing else, so it would look perfectly
        // usable — and would save nothing, because the answer is an id.
        assert_eq!(sharpen.usable, Usable::Asynchronous);
        assert_eq!(sharpen.settings, Settings::None);
        // No summary in the description: the path has to do.
        assert_eq!(sharpen.summary, "sharpen");
    }

    #[test]
    fn a_description_this_does_not_understand_yields_nothing_rather_than_failing() {
        assert!(tools(&Value::Null).is_empty());
        assert!(tools(&serde_json::json!({})).is_empty());
        assert!(tools(&serde_json::json!({ "paths": 42 })).is_empty());
        assert!(tools(&serde_json::json!({ "paths": { "/x": { "post": {} } } })).is_empty());
    }

    /// The shape FastAPI writes: the body schema stands under
    /// `components/schemas` and the operation only points at it.
    fn fastapi() -> Value {
        serde_json::json!({
            "openapi": "3.1.0",
            "paths": {
                "/upload/compress": { "post": {
                    "tags": ["tools"],
                    "summary": "Compress",
                    "requestBody": { "content": { "multipart/form-data": {
                        "schema": { "$ref": "#/components/schemas/Body_compress_upload_compress_post" }
                    }}},
                    "responses": { "200": {} }
                }},
                "/upload/convert": { "post": {
                    "summary": "Convert",
                    "requestBody": { "content": { "multipart/form-data": {
                        "schema": { "$ref": "#/components/schemas/Body_convert_upload_convert_post" }
                    }}},
                    "responses": { "200": {} }
                }}
            },
            "components": { "schemas": {
                "Body_compress_upload_compress_post": {
                    "type": "object",
                    "required": ["file"],
                    "properties": {
                        "file": { "type": "string", "format": "binary" },
                        "quality": { "$ref": "#/components/schemas/Quality" }
                    }
                },
                "Body_convert_upload_convert_post": {
                    "type": "object",
                    "required": ["file"],
                    "properties": { "file": { "type": "string", "format": "binary" } }
                },
                "Quality": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Output quality"
                }
            }}
        })
    }

    #[test]
    fn a_body_that_only_stands_somewhere_else_is_followed_to_where_it_stands() {
        // Measured before the reference was followed: 0 tools out of 2
        // endpoints, and a service that came out empty without saying why.
        let found = tools(&fastapi());
        assert_eq!(found.len(), 2);

        let compress = found.iter().find(|t| t.summary == "Compress").unwrap();
        assert_eq!(compress.file_field, "file");
        assert_eq!(compress.usable, Usable::Yes);

        // The option is a reference of its own, and the form has to know it is
        // a number with a range rather than a box for anything at all.
        let Settings::Fields { field, fields } = &compress.settings else {
            panic!("expected fields, got {:?}", compress.settings);
        };
        assert_eq!(*field, None, "its own form field, not a box holding JSON");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "quality");
        assert_eq!(
            fields[0].kind,
            FieldKind::Number {
                minimum: Some(1.0),
                maximum: Some(100.0)
            }
        );
        assert_eq!(fields[0].description.as_deref(), Some("Output quality"));
    }

    #[test]
    fn a_schema_written_in_parts_is_read_as_the_whole_it_describes() {
        let spec = serde_json::json!({
            "paths": { "/convert": { "post": {
                "summary": "Convert",
                "requestBody": { "content": { "multipart/form-data": { "schema": {
                    "allOf": [
                        { "$ref": "#/components/schemas/Upload" },
                        { "properties": {
                            "format": { "type": "string", "enum": ["png", "webp"] }
                        }}
                    ],
                    "required": ["format"]
                }}}},
                "responses": { "200": {} }
            }}},
            "components": { "schemas": { "Upload": {
                "type": "object",
                "required": ["file"],
                "properties": {
                    "file": { "type": "string", "format": "binary" },
                    "clientJobId": { "type": "string" }
                }
            }}}
        });

        let found = tools(&spec);
        assert_eq!(found.len(), 1);
        // The file comes from the shared part, the option from the specific
        // one, and what each part calls required counts in both.
        assert_eq!(found[0].file_field, "file");
        assert_eq!(found[0].usable, Usable::NeedsSettings);

        let Settings::Fields { fields, .. } = &found[0].settings else {
            panic!("expected fields, got {:?}", found[0].settings);
        };
        assert_eq!(fields.len(), 1, "the plumbing stays out");
        assert_eq!(
            fields[0].kind,
            FieldKind::Choice(vec!["png".into(), "webp".into()])
        );
        assert!(fields[0].required);
    }

    #[test]
    fn a_reference_that_leads_in_a_circle_costs_the_endpoint_and_nothing_else() {
        let spec = serde_json::json!({
            "paths": {
                "/ring": { "post": {
                    "requestBody": { "content": { "multipart/form-data": {
                        "schema": { "$ref": "#/components/schemas/Ring" }
                    }}},
                    "responses": { "200": {} }
                }},
                "/away": { "post": {
                    "requestBody": { "content": { "multipart/form-data": {
                        "schema": { "$ref": "other.json#/components/schemas/Body" }
                    }}},
                    "responses": { "200": {} }
                }},
                "/missing": { "post": {
                    "requestBody": { "content": { "multipart/form-data": {
                        "schema": { "$ref": "#/components/schemas/NotThere" }
                    }}},
                    "responses": { "200": {} }
                }}
            },
            "components": { "schemas": {
                "Ring": { "$ref": "#/components/schemas/Ring" }
            }}
        });

        // A ring, a reference into another file, and one that names nothing:
        // each costs its own endpoint, and none of them the program.
        assert!(tools(&spec).is_empty());
    }

    #[test]
    fn every_form_field_the_description_names_survives_the_reading() {
        // A service that declares each option as a form field of its own:
        // nothing is called settings, nothing is an object. Alphabetical order
        // put `height` first and `width` -- required -- was dropped, which made
        // a tool that shows a tick and is refused on every use.
        let spec = serde_json::json!({
            "paths": { "/resize": { "post": {
                "summary": "Resize",
                "requestBody": { "content": { "multipart/form-data": { "schema": {
                    "type": "object",
                    "required": ["file", "width"],
                    "properties": {
                        "file": { "type": "string", "format": "binary" },
                        "height": { "type": "integer", "minimum": 1, "description": "Pixels" },
                        "width": { "type": "integer", "minimum": 1 },
                        "clientJobId": { "type": "string" }
                    }
                }}}},
                "responses": { "200": { "description": "ok" } }
            }}}
        });

        let found = tools(&spec);
        let Settings::Fields { field, fields } = &found[0].settings else {
            panic!("expected fields, got {:?}", found[0].settings);
        };
        assert_eq!(*field, None, "no single field holds them");

        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["height", "width"], "the plumbing stays out");
        assert!(fields.iter().find(|f| f.name == "width").unwrap().required);
        assert!(!fields.iter().find(|f| f.name == "height").unwrap().required);
        // And the tool says so: a required field nobody has filled in yet.
        assert_eq!(found[0].usable, Usable::NeedsSettings);
    }

    #[test]
    fn a_second_file_is_not_offered_as_something_to_type() {
        let spec = serde_json::json!({
            "paths": { "/watermark": { "post": {
                "summary": "Watermark",
                "requestBody": { "content": { "multipart/form-data": { "schema": {
                    "required": ["file", "overlay"],
                    "properties": {
                        "file": { "type": "string", "format": "binary" },
                        "overlay": { "type": "string", "format": "binary" }
                    }
                }}}},
                "responses": { "200": { "description": "ok" } }
            }}}
        });

        let found = tools(&spec);
        // This program sends one file. A box asking for the second one in
        // words is worse than saying there is nothing to fill in.
        assert_eq!(found[0].settings, Settings::None);
        assert_eq!(found[0].usable, Usable::NeedsSettings, "it wants two files");
    }

    #[test]
    fn a_field_that_may_also_be_null_keeps_the_type_it_declares() {
        // OpenAPI 3.1 and JSON Schema 2020-12 write nullable as a list of
        // types. Read as a single word -- which it is not -- every one of these
        // became a free text box, and the range beside it went unused.
        let spec = serde_json::json!({
            "paths": { "/compress": { "post": {
                "summary": "Compress",
                "requestBody": { "content": { "multipart/form-data": { "schema": {
                    "required": ["file"],
                    "properties": {
                        "file": { "type": ["string", "null"], "format": "binary" },
                        "options": { "type": "object", "properties": {
                            "quality": {
                                "type": ["integer", "null"],
                                "minimum": 1,
                                "maximum": 100
                            },
                            "strip": { "type": ["boolean", "null"] },
                            "note": { "type": ["null", "string"] }
                        }}
                    }
                }}}},
                "responses": { "200": { "description": "ok" } }
            }}}
        });

        let found = tools(&spec);
        // The file field is written the same way and is still the file.
        assert_eq!(found[0].file_field, "file");

        let Settings::Fields { fields, .. } = &found[0].settings else {
            panic!("expected fields, got {:?}", found[0].settings);
        };
        let kind = |name: &str| {
            fields
                .iter()
                .find(|f| f.name == name)
                .unwrap_or_else(|| panic!("{name} is missing"))
                .kind
                .clone()
        };
        assert_eq!(
            kind("quality"),
            FieldKind::Number {
                minimum: Some(1.0),
                maximum: Some(100.0)
            }
        );
        assert_eq!(kind("strip"), FieldKind::Flag);
        assert_eq!(kind("note"), FieldKind::Text);
    }

    #[test]
    fn the_base_the_description_names_is_kept_for_every_tool() {
        // Nothing said: the same as `/`, and the address stays what it was.
        assert!(tools(&spec()).iter().all(|tool| tool.base.is_empty()));

        let mut spec = spec();
        spec["servers"] = serde_json::json!([
            { "url": "/api/v1" },
            { "url": "https://elsewhere.example" }
        ]);
        // The first entry only: the list is written as alternatives.
        assert!(tools(&spec).iter().all(|tool| tool.base == "/api/v1"));

        // An address with variables in it is not an address yet, and a guessed
        // host answers nothing at all.
        spec["servers"] = serde_json::json!([{ "url": "https://{region}.example.com/v1" }]);
        assert!(tools(&spec).iter().all(|tool| tool.base.is_empty()));
    }
    // The one field a line declares, so a test can speak about it in one breath.
    fn one(text: &str) -> Field {
        let mut fields = fields_from_prose(text);
        assert_eq!(fields.len(), 1, "expected one field from {text:?}");
        fields.remove(0)
    }

    fn names(text: &str) -> Vec<String> {
        fields_from_prose(text)
            .into_iter()
            .map(|field| field.name)
            .collect()
    }

    #[test]
    fn the_list_a_schema_generator_writes_becomes_one_field_per_line() {
        let fields = fields_from_prose(
            "JSON string with options:\n\
         - `left` (number, required) - Left offset in pixels (min 0)\n\
         - `unit` (string, optional) - One of: px, percent\n\
         - `withoutEnlargement` (boolean, default false) - Prevent upscaling\n",
        );

        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "left");
        assert!(fields[0].required);
        assert_eq!(
            fields[0].kind,
            FieldKind::Number {
                minimum: Some(0.0),
                maximum: None
            }
        );
        assert_eq!(
            fields[0].description.as_deref(),
            Some("Left offset in pixels (min 0)")
        );
        assert!(!fields[1].required);
        assert_eq!(
            fields[1].kind,
            FieldKind::Choice(vec!["px".into(), "percent".into()])
        );
        assert_eq!(fields[2].kind, FieldKind::Flag);
    }

    #[test]
    fn a_field_declared_as_a_number_keeps_the_range_the_prose_names() {
        for (line, minimum, maximum) in [
            ("- `q` (number 0-100) - x", Some(0.0), Some(100.0)),
            ("- `q` (number, 0-100) - x", Some(0.0), Some(100.0)),
            ("- `q` (number -100 to 100) - x", Some(-100.0), Some(100.0)),
            ("- `q` (number -80 to -20) - x", Some(-80.0), Some(-20.0)),
            (
                "- `q` (number 0.05-1, default 0.3) - x",
                Some(0.05),
                Some(1.0),
            ),
            ("- `q` (integer >= 16) - x", Some(16.0), None),
            ("- `q` (number, min 10, optional) - x", Some(10.0), None),
            ("- `q` (number, max 100) - x", None, Some(100.0)),
            ("- `q` (0-100) - Quality", Some(0.0), Some(100.0)),
            (
                "- `q` (number, required) - Quality (min 0)",
                Some(0.0),
                None,
            ),
            (
                "- `q` (number) - Output quality (1-100)",
                Some(1.0),
                Some(100.0),
            ),
        ] {
            assert_eq!(
                one(line).kind,
                FieldKind::Number { minimum, maximum },
                "{line}"
            );
        }
    }

    #[test]
    fn a_range_whose_ends_are_the_wrong_way_round_is_no_range_at_all() {
        // Somebody misread something, here or in the prose. A spinner that accepts
        // no value at all is worse than one without limits.
        assert_eq!(
            one("- `q` (number, min 5, max 1) - x").kind,
            FieldKind::Number {
                minimum: None,
                maximum: None
            }
        );
    }

    #[test]
    fn a_number_the_prose_gives_no_range_for_gets_none_rather_than_a_guess() {
        for line in [
            "- `angle` (number, optional) - Rotation angle in degrees",
            "- `maxWidth` (integer, default 0) - Max width (0 = no limit)",
            "- `chunk` (integer, optional) - Every nth page (0 keeps them together)",
        ] {
            assert_eq!(
                one(line).kind,
                FieldKind::Number {
                    minimum: None,
                    maximum: None
                },
                "{line}"
            );
        }
    }

    #[test]
    fn a_length_limit_on_a_text_field_is_not_turned_into_a_range_of_values() {
        for line in [
            "- `pw` (string 1-256 chars, required) - Password",
            "- `title` (string, optional, max 500) - Document title",
            "- `text` (string, required) - Watermark text (1-200 characters)",
        ] {
            assert_eq!(one(line).kind, FieldKind::Text, "{line}");
        }
    }

    #[test]
    fn a_list_of_values_becomes_a_choice_however_the_service_writes_it() {
        let wanted = FieldKind::Choice(vec!["px".into(), "percent".into()]);
        for line in [
            "- `unit` (string) - One of: px, percent",
            "* `unit` (string): one of px, percent",
            "- **unit** (string) -- one of `px` | `percent`",
            "- `unit` (string) - Unit of the offsets, one of: px or percent",
            "- `unit` (optional) - One of: px, percent",
        ] {
            assert_eq!(one(line).kind, wanted, "{line}");
        }
    }

    #[test]
    fn a_remark_or_a_sentence_behind_a_value_stays_out_of_the_value() {
        assert_eq!(
        one("- `mode` (string, default \"subject\") - One of: subject, attention (alias for subject), trim").kind,
        FieldKind::Choice(vec!["subject".into(), "attention".into(), "trim".into()])
    );
        assert_eq!(
        one("- `lang` (string, default \"auto\") - One of: auto, en, ko. Korean is unsupported by the fast tier").kind,
        FieldKind::Choice(vec!["auto".into(), "en".into(), "ko".into()])
    );
        assert_eq!(
            one("- `target` (string, default \"9:16\") - One of: 16:9, 9:16, 1:1").kind,
            FieldKind::Choice(vec!["16:9".into(), "9:16".into(), "1:1".into()])
        );
    }

    #[test]
    fn one_unreadable_value_drops_the_whole_list_rather_than_hiding_an_option() {
        assert_eq!(
            one("- `bg` (string) - One of: transparent, a hex colour of your choosing").kind,
            FieldKind::Text
        );
    }

    #[test]
    fn a_sentence_that_merely_mentions_other_fields_is_not_a_list_of_values() {
        // "one of `preset`, `tier`" names two other fields. Offering their names
        // as the values of this one would send a word the service never heard of.
        assert_eq!(
            one("- `mode` (string, required) - Mutually exclusive with one of `preset`, `tier`")
                .kind,
            FieldKind::Text
        );
    }

    #[test]
    fn a_number_that_lists_its_allowed_values_stays_a_number() {
        // A choice travels as a string, and an endpoint that asked for 90 refuses
        // "90".
        assert_eq!(
            one("- `angle` (number, default 0) - Rotation angle, one of: 90, 180, 270").kind,
            FieldKind::Number {
                minimum: None,
                maximum: None
            }
        );
    }

    #[test]
    fn the_bullet_the_markup_and_the_separator_may_all_be_written_differently() {
        for line in [
            "- `left` (number, required) - Left offset",
            "* `left` (number, required) - Left offset",
            "+ `left` (number, required) - Left offset",
            "\u{2022} `left` (number, required) - Left offset",
            "\u{2013} `left` (number, required) - Left offset",
            "1. `left` (number, required) - Left offset",
            "2) `left` (number, required) - Left offset",
            "- **left** (number, required) - Left offset",
            "- __left__ (number, required) - Left offset",
            "- *left* (number, required) - Left offset",
            "- **`left`** (number, required) - Left offset",
            "- `left` *(number, required)* - Left offset",
            "- left (number, required) - Left offset",
            "- `left` (number, required): Left offset",
            "- `left` (number, required) -- Left offset",
            "- `left` (number, required) \u{2014} Left offset",
            "- `left`* (number) - Left offset",
            "- `left*` (number) - Left offset",
        ] {
            let field = one(line);
            assert_eq!(field.name, "left", "{line}");
            assert!(field.required, "{line}");
            assert_eq!(field.description.as_deref(), Some("Left offset"), "{line}");
        }
    }

    #[test]
    fn a_declaration_without_a_bracket_is_read_when_the_sentence_declares_instead() {
        assert_eq!(
            one("- `quality` - Output quality. Default: 80").kind,
            FieldKind::Number {
                minimum: None,
                maximum: None
            }
        );
        assert_eq!(one("- `keep` - Defaults: true").kind, FieldKind::Flag);
        assert_eq!(
            one("- `align`: One of: left, center, right").kind,
            FieldKind::Choice(vec!["left".into(), "center".into(), "right".into()])
        );
    }

    #[test]
    fn a_name_with_nothing_to_vouch_for_it_is_left_alone() {
        // The shape of every glossary in every piece of documentation: a marked
        // name, a dash, an explanation. Reading these as fields would put three
        // invented inputs on screen and take the JSON box away.
        assert!(
            fields_from_prose(
                "Output format for the rendered page.\n\n\
             Supported values:\n\
             - `png` - Lossless, supports transparency\n\
             - `jpeg` - Smaller files, no transparency\n\
             - `webp` - Modern format, good compression\n"
            )
            .is_empty()
        );
        assert!(
            fields_from_prose(
                "Notes:\n\
             - **Important** - the format is locked to PNG\n\
             - *Tip* - use `width` to set the size\n\
             - `E_TOO_LARGE` - the file exceeds 10 MB\n\
             - `resize` - scales the image\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn a_sentence_that_happens_to_start_with_a_word_and_a_bracket_declares_nothing() {
        // Without a separator behind the bracket the line is a sentence about a
        // value, not the declaration of a setting — and the name would be sent
        // capitalised, which no service knows.
        assert!(
            fields_from_prose(
                "Notes:\n\
             - Quality (integer) defaults to 82 for JPEG output\n\
             - Transparency (boolean) is preserved for PNG\n\
             - Position (string) is one of: top-left, top-right\n\
             - Width (number, max 8000) is capped for free accounts\n\
             - Note (string): this is not a setting\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn prose_that_lists_no_fields_yields_nothing_and_leaves_the_text_box_alone() {
        for text in [
            "",
            "---",
            "JSON string with options:",
            "Dedicated JPG to PNG converter using the convert pipeline.",
            "Optional settings: quality (number 1-100). The output format is locked by the endpoint.",
            "- Supports PNG, JPEG and WebP\n- Maximum file size is 10 MB",
            "- Choose one of: png, jpg, webp",
            "- `width` and `height` must both be given\n- 1920x1080 is a good starting point",
            "- `png` (default) - Portable Network Graphics",
        ] {
            assert!(fields_from_prose(text).is_empty(), "{text:?}");
        }
    }

    #[test]
    fn a_declaration_no_single_input_can_hold_gives_up_the_whole_list() {
        // Reading the rest and dropping this one would build a form that cannot
        // send `terms` at all — worse than the free-text box, where the user can
        // still type the array by hand.
        let text = "JSON string with options:\n\
                - `terms` (string[], required) - Words to redact\n\
                - `caseSensitive` (boolean, default false) - Match case\n";
        assert!(fields_from_prose(text).is_empty());

        for line in [
            "- `region` (object, optional) - Restrict to a rectangle",
            "- `stops` (array, optional) - Gradient stops",
            "- `boxes` (array of {id, text}) - Text for each position",
            "- `steps[].toolId` (string, required) - Tool ID for this step",
        ] {
            assert!(fields_from_prose(line).is_empty(), "{line}");
        }
    }

    #[test]
    fn what_belongs_inside_an_object_is_not_offered_beside_it() {
        // The sub-bullets describe the object, and the object itself already ends
        // the reading — but the same must hold when they are the only thing that
        // is indented.
        let text = "JSON string with options:\n\
                - `blockSize` (integer 2-128, default 12) - Pixel block size\n\
                - `deep` (boolean, default false) - Use the slow model\n  \
                  - `left` (integer >= 0) - Left offset\n";
        assert_eq!(
            names(text),
            vec!["blockSize".to_string(), "deep".to_string()]
        );
    }

    #[test]
    fn a_note_further_down_does_not_decide_what_counts_as_the_outermost_list() {
        // The list is indented as a whole and the note is not. Measuring the
        // outermost level across the whole text would drop every field here.
        let text = "Options:\n  \
                - `width` (number, optional) - Viewport width\n  \
                - `height` (number, optional) - Viewport height\n\
                \n\
                - All sizes are CSS pixels.\n";
        assert_eq!(names(text), vec!["width".to_string(), "height".to_string()]);
    }

    #[test]
    fn required_is_only_believed_where_it_stands_for_the_field_itself() {
        let fields = fields_from_prose(
            "- `left` (number, required) - Left offset\n\
         - `unit` (string, optional) - Unit\n\
         - `fit` (string, default \"contain\") - Fit\n\
         - `note` (string) - Free text\n\
         - `everyN` (integer, optional) - Chunk size (required when mode is \"every\")\n\
         - `bg` (string) - Required for JPEG output, ignored otherwise\n\
         - `mode` (string) - Required. The thing to do\n",
        );
        let required: Vec<bool> = fields.iter().map(|field| field.required).collect();
        assert_eq!(
            required,
            vec![true, false, false, false, false, false, false]
        );
    }

    #[test]
    fn the_same_field_named_twice_is_taken_once() {
        let fields = fields_from_prose(
            "- `format` (string) - Output format\n- `format` (number 1-9) - Again",
        );
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].kind, FieldKind::Text);
    }

    #[test]
    fn a_service_that_declares_its_options_in_german_is_read_as_well() {
        let fields = fields_from_prose(
            "JSON-Zeichenkette mit Einstellungen:\n\
         - `breite` (Zahl, erforderlich) - Zielbreite in Pixeln\n\
         - `guete` (Ganzzahl 1-100, Standard: 80) - Ausgabequalitaet\n\
         - `metadaten` (Wahrheitswert, optional) - Metadaten behalten\n\
         - `ausrichtung` (Zeichenkette) - Eines von: quer, hoch\n",
        );
        let kinds: Vec<&FieldKind> = fields.iter().map(|field| &field.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &FieldKind::Number {
                    minimum: None,
                    maximum: None
                },
                &FieldKind::Number {
                    minimum: Some(1.0),
                    maximum: Some(100.0)
                },
                &FieldKind::Flag,
                &FieldKind::Choice(vec!["quer".into(), "hoch".into()]),
            ]
        );
        assert!(fields[0].required);
    }

    #[test]
    fn text_that_is_not_ascii_is_read_without_falling_over() {
        // Lowercasing is not length-preserving. Indexing the original string with
        // an offset found in a lowercased copy would slice into a character.
        let field = one("- `size` (number) - Gr\u{f6}\u{df}e in Pixeln. Default: 80");
        assert_eq!(field.name, "size");
        assert!(fields_from_prose("- \u{130}I\u{130} ist keine Deklaration").is_empty());
        assert!(
            fields_from_prose("- \u{4f60}\u{597d} (\u{6570}\u{5b57}) - \u{6d4b}\u{8bd5}")
                .is_empty()
        );
    }

    #[test]
    fn a_line_of_any_length_is_read_without_running_out_of_stack() {
        let long = format!("- `x` ({}) - y", "zzz ".repeat(50_000));
        assert!(fields_from_prose(&long).is_empty());
        let deep = format!("- `x` ({}) - y", "(".repeat(20_000));
        assert!(fields_from_prose(&deep).is_empty());
    }
}
