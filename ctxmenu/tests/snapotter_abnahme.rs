//! Acceptance test against the real service description this program was built
//! against. Function before solution: whatever changes inside `service::spec`,
//! these numbers must not get smaller and these addresses must not move.
//!
//! The description is a copy of `http://192.168.2.11:1349/api/docs/openapi.json`
//! taken on 2026-08-16, checked in beside this file so the test needs no network
//! and runs on a build machine that has never seen that LAN. It is OpenAPI 3.1
//! with `servers: [{"url": "/"}]`, 351 paths and 258 inline multipart schemas --
//! the shape that matters here.

use std::path::PathBuf;

use ctxmenu::favourites::{Tool, WebMode};
use ctxmenu::service::{self, spec};

fn spec_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("snapotter-openapi.json")
}

fn snapotter() -> serde_json::Value {
    let raw = std::fs::read(spec_path()).unwrap_or_else(|e| {
        panic!(
            "the saved description is missing at {:?}: {e}. \
             Fetch it again with: curl http://192.168.2.11:1349/api/docs/openapi.json -o ctxmenu/tests/data/snapotter-openapi.json",
            spec_path()
        )
    });
    serde_json::from_slice(&raw).expect("the saved description is valid JSON")
}

fn service_entry() -> service::Service {
    service::Service {
        id: "snapotter".into(),
        name: "SnapOtter".into(),
        spec_url: "http://192.168.2.11:1349/api/docs/openapi.json".into(),
        // Deliberately no real key: this test reads a file and sends nothing.
        auth_header: Some(ctxmenu::favourites::Header {
            name: "Authorization".into(),
            value: "Bearer test".into(),
        }),
        allow_insecure: true,
        result_path: "downloadUrl".into(),
    }
}

/// The tool count is the headline number of the services tab.
///
/// Measured on 2026-08-16 against this description: **232 tools** out of 351
/// paths. The description carries 258 inline multipart schemas; the 26 that do
/// not become tools either use a verb without a body or declare no field of
/// `type: string, format: binary`, and both are correct to skip.
///
/// A change that finds *more* is an improvement and may raise this floor; a
/// change that finds fewer has broken the tab.
#[test]
fn the_description_still_yields_every_tool_it_did() {
    let tools = spec::tools(&snapotter());
    assert!(
        tools.len() >= 232,
        "the services tab found {} tools, it used to find 232 -- something in \
         the parser stopped recognising endpoints",
        tools.len()
    );
}

/// The address a favourite ends up sending to.
///
/// This is the one that breaks quietly: `servers: [{"url": "/"}]` plus a path
/// of `/api/v1/...` must stay `http://192.168.2.11:1349/api/v1/...`, with one
/// slash, not two. Any handling of the `servers` block has to survive this.
#[test]
fn the_endpoint_of_a_known_tool_is_unchanged() {
    let tools = spec::tools(&snapotter());
    let compress = tools
        .iter()
        .find(|t| t.path == "/api/v1/tools/image/compress")
        .expect("the compress endpoint is in this description");

    let favourite = service::favourite_for(&service_entry(), compress, None, ".min");
    let Tool::Web(web) = &favourite.tool else {
        panic!("a service tool is always a web tool");
    };
    let WebMode::Upload(upload) = &web.mode else {
        panic!("a service tool always uploads");
    };

    assert_eq!(
        upload.endpoint, "http://192.168.2.11:1349/api/v1/tools/image/compress",
        "the endpoint moved -- a favourite built from this service would now \
         send to the wrong address"
    );
    assert_eq!(upload.method, "POST");
}

/// The way back to a job the service only took in.
///
/// Measured on 2026-08-16: the same upload of the same picture was answered
/// `202 {"jobId": …, "async": true}` in four of six rounds and with the finished
/// result in two, so this is not a property of the endpoint and cannot be read
/// off its own responses. What the description does say is where a job is asked
/// after — `/api/v1/jobs/{jobId}/progress`, a `GET` answered as Server-Sent
/// Events — and every favourite built from this service carries it. Should that
/// path move or stop being recognised, two thirds of the clicks go back to
/// failing.
#[test]
fn every_tool_knows_where_a_queued_job_is_asked_after() {
    let description = snapotter();
    assert_eq!(
        spec::progress_path(&description),
        "/api/v1/jobs/{jobId}/progress"
    );

    let tools = spec::tools(&description);
    let compress = tools
        .iter()
        .find(|t| t.path == "/api/v1/tools/image/compress")
        .expect("the compress endpoint is in this description");

    let favourite = service::favourite_for(&service_entry(), compress, None, ".min");
    let Tool::Web(web) = &favourite.tool else {
        panic!("a service tool is always a web tool");
    };
    let WebMode::Upload(upload) = &web.mode else {
        panic!("a service tool always uploads");
    };

    let poll = upload
        .poll
        .as_ref()
        .expect("this description says where to ask");
    assert_eq!(poll.path, "/api/v1/jobs/{jobId}/progress");
    assert_eq!(poll.job, "jobId");

    // Every one of them, not just the one that was looked at.
    for tool in &tools {
        assert_eq!(
            tool.progress, "/api/v1/jobs/{jobId}/progress",
            "{} would have nowhere to ask after a job",
            tool.path
        );
    }
}

/// No address may grow a double slash or lose its host.
#[test]
fn every_endpoint_stays_a_single_well_formed_address() {
    let service = service_entry();
    let tools = spec::tools(&snapotter());

    for tool in &tools {
        let favourite = service::favourite_for(&service, tool, None, ".min");
        let Tool::Web(web) = &favourite.tool else {
            continue;
        };
        let WebMode::Upload(upload) = &web.mode else {
            continue;
        };

        let address = &upload.endpoint;
        assert!(
            address.starts_with("http://192.168.2.11:1349/"),
            "{address} does not sit under the service host"
        );
        assert!(
            !address["http://".len()..].contains("//"),
            "{address} carries a double slash"
        );
        assert!(
            !address.ends_with('/') || tool.path == "/",
            "{address} ends in a slash it did not have"
        );
    }
}

/// The file field is what the upload is hung on; without it nothing is sent.
#[test]
fn every_tool_names_the_field_the_file_goes_into() {
    for tool in spec::tools(&snapotter()) {
        assert!(
            !tool.file_field.trim().is_empty(),
            "{} has no file field",
            tool.path
        );
    }
}

/// The ids that ended up in the user's `favourites.json` must keep resolving to
/// the same tools, or their context menu entries stop working.
#[test]
fn the_ids_the_user_already_has_still_come_out_of_this_description() {
    let service = service_entry();
    let tools = spec::tools(&snapotter());

    // Taken from a real favourites.json on 2026-08-16.
    let wanted = [
        "snapotter__compress_image",
        "snapotter__crop_image",
        "snapotter__color_adjustments",
        "snapotter__gif_to_jpg",
    ];

    let built: Vec<String> = tools
        .iter()
        .map(|tool| service::favourite_for(&service, tool, None, ".min").id)
        .collect();

    for id in wanted {
        assert!(
            built.iter().any(|b| b == id),
            "the id {id} is in the user's favourites but this description no \
             longer produces it -- their menu entry would stop finding its tool"
        );
    }
}
