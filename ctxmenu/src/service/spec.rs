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

use serde_json::Value;

/// One endpoint that takes a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    /// `/api/v1/tools/image/compress`
    pub path: String,
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
    Fields { field: String, fields: Vec<Field> },
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
            if let Some(tool) = read_operation(path, method, operation) {
                out.push(tool);
            }
        }
    }
    out
}

fn read_operation(path: &str, method: &str, operation: &Value) -> Option<Tool> {
    let schema = operation
        .get("requestBody")?
        .get("content")?
        .get("multipart/form-data")?
        .get("schema")?;
    let properties = schema.get("properties")?.as_object()?;

    // The file field: a string declared as binary. Without one this endpoint
    // is not something a right-click on a file can serve.
    let (file_field, _) = properties.iter().find(|(_, value)| {
        value.get("type").and_then(Value::as_str) == Some("string")
            && value.get("format").and_then(Value::as_str) == Some("binary")
    })?;

    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let settings = read_settings(properties, file_field, &required);
    let usable = match (&settings, answers_asynchronously(operation)) {
        (_, true) => Usable::Asynchronous,
        (Settings::None, _) => Usable::Yes,
        // A field that is not required can be left out, so the tool still
        // works with the file alone.
        (Settings::Text { field, .. } | Settings::Fields { field, .. }, _)
            if !required.contains(&field.as_str()) =>
        {
            Usable::Yes
        }
        _ => Usable::NeedsSettings,
    };

    Some(Tool {
        path: path.to_string(),
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
    properties: &serde_json::Map<String, Value>,
    file_field: &str,
    required: &[&str],
) -> Settings {
    let candidates: Vec<(&String, &Value)> = properties
        .iter()
        .filter(|(name, _)| name.as_str() != file_field)
        .filter(|(name, _)| !PLUMBING.contains(&name.to_lowercase().as_str()))
        .collect();

    // A name the service itself uses for its options beats alphabetical luck;
    // a described object beats a bare string; otherwise the first one left.
    let picked = SETTINGS_NAMES
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
        })
        .or_else(|| candidates.first());

    let Some((name, value)) = picked else {
        return Settings::None;
    };
    let (name, value) = (*name, *value);

    // A described object is the good case: every property becomes an input.
    if let Some(fields) = read_fields(value, required) {
        return Settings::Fields {
            field: name.clone(),
            fields,
        };
    }

    Settings::Text {
        field: name.clone(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// The properties of a described object, if this is one.
fn read_fields(value: &Value, outer_required: &[&str]) -> Option<Vec<Field>> {
    let properties = value.get("properties")?.as_object()?;
    let required: Vec<&str> = value
        .get("required")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_else(|| outer_required.to_vec());

    let mut fields = Vec::new();
    for (name, property) in properties {
        let kind = match property.get("type").and_then(Value::as_str) {
            _ if property.get("enum").is_some() => FieldKind::Choice(
                property
                    .get("enum")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .map(|v| match v {
                                Value::String(text) => text.clone(),
                                other => other.to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            Some("integer" | "number") => FieldKind::Number {
                minimum: property.get("minimum").and_then(Value::as_f64),
                maximum: property.get("maximum").and_then(Value::as_f64),
            },
            Some("boolean") => FieldKind::Flag,
            _ => FieldKind::Text,
        };

        fields.push(Field {
            name: name.clone(),
            kind,
            required: required.contains(&name.as_str()),
            description: property
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    Some(fields)
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

fn last_segment(path: &str) -> &str {
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(path)
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
        assert_eq!(field, "options");

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
}
