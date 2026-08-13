//! End-to-end proof for milestone 3: create, scan, back up, delete, restore.
//!
//! Everything happens under `HKCU\SOFTWARE\Classes\Directory\shell\` with key
//! names that are unmistakably this tool's own. HKCU needs no elevation and
//! the fixtures remove themselves, so no VM is required — that requirement
//! from ToDo 2.8 applies to HKLM tests, which none of these are.
//!
//! The fixture names carry the awkward cases from ToDo 16 on purpose: spaces,
//! umlauts and an `&`, which becomes a menu accelerator and is a classic
//! quoting trap on the way through `reg.exe`.

use ctxmenu::model::{Category, EntryKind, Scope};
use ctxmenu::registry::paths::RegTarget;
use ctxmenu::registry::scan::{ScanOptions, scan};
use ctxmenu::registry::{backup, write};
use windows_registry::CURRENT_USER;

/// These tests create and delete keys under the same `Directory\shell` that
/// they also scan, so they must not run at the same time as one another.
static REGISTRY: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Takes the lock, ignoring poisoning: a failed test leaves the registry
/// cleaned up by `Fixture::drop`, so the following test is still safe to run
/// and should report its own result rather than an inherited panic.
fn serialized() -> std::sync::MutexGuard<'static, ()> {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner())
}

const DISPLAY: &str = "Selbsttest mit Leerzeichen & Ümläut";
const ICON: &str = r"%SystemRoot%\system32\shell32.dll,-244";
const COMMAND: &str = r#""C:\Program Files\Test Tool\t.exe" "%V""#;

/// A throwaway verb key that deletes itself when the test ends.
///
/// `Drop` rather than cleanup at the end of the test body: a failing assertion
/// unwinds, and leaving a stray entry in the user's real context menu would be
/// a rude way to report a test failure.
struct Fixture {
    target: RegTarget,
}

impl Fixture {
    fn create(name: &str) -> Self {
        let target = RegTarget {
            scope: Scope::User,
            relative: format!(r"Directory\shell\{name}"),
        };

        let key = CURRENT_USER
            .create(target.key_path())
            .expect("HKCU\\SOFTWARE\\Classes is writable without elevation");
        key.set_string("", DISPLAY).expect("default value");
        key.set_string("Icon", ICON).expect("icon reference");
        // Presence flag: the value is empty, only its existence matters.
        key.set_string("Extended", "").expect("extended flag");

        CURRENT_USER
            .create(format!("{}\\command", target.key_path()))
            .expect("command subkey")
            .set_string("", COMMAND)
            .expect("command line");

        Self { target }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = CURRENT_USER.remove_tree(self.target.key_path());
    }
}

fn find_entry(key_name: &str) -> Option<ctxmenu::model::ContextEntry> {
    let options = ScanOptions {
        scopes: vec![Scope::User],
        categories: Some(vec![Category::Directory]),
        ..ScanOptions::default()
    };
    scan(&options, |_| {})
        .entries
        .into_iter()
        .find(|e| e.key_name == key_name)
}

#[test]
fn the_scanner_reads_every_field_of_a_known_key() {
    let _guard = serialized();
    let name = "ctxmenu selftest scan & Ümläut";
    let fixture = Fixture::create(name);

    let entry = find_entry(name).expect("the fixture must show up in a Directory scan");

    assert_eq!(entry.display_name, DISPLAY);
    assert_eq!(entry.icon_ref.as_deref(), Some(ICON));
    assert!(entry.extended, "Extended is a presence flag");
    assert!(!entry.hidden);
    assert_eq!(entry.scope, Scope::User);
    assert!(
        !entry.read_only,
        "a key the test just created in HKCU must be writable"
    );
    assert_eq!(entry.registry_path, fixture.target.full_path());

    match entry.kind {
        EntryKind::Verb { command, .. } => {
            assert_eq!(command.as_deref(), Some(COMMAND));
        }
        other => panic!("expected a verb, got {other:?}"),
    }
}

#[test]
fn a_deleted_key_comes_back_from_its_backup() {
    let _guard = serialized();
    let name = "ctxmenu selftest restore & Ümläut";
    let fixture = Fixture::create(name);
    let target = fixture.target.clone();

    assert!(write::exists(&target), "fixture should exist to begin with");

    // 1. Back up before touching anything.
    let token = backup::export("selftest", std::slice::from_ref(&target))
        .expect("export of an existing key");
    let directory = token.directory().to_path_buf();

    let manifest = backup::read_manifest(&directory).expect("manifest.json");
    assert_eq!(manifest.entries.len(), 1);
    assert!(manifest.missing.is_empty());
    assert_eq!(manifest.entries[0].registry_path, target.full_path());
    assert!(
        directory.join(&manifest.entries[0].file).exists(),
        "the exported .reg file must be on disk"
    );

    // 2. Delete.
    write::delete_tree(&target, &token).expect("covered delete");
    assert!(!write::exists(&target));
    assert!(
        find_entry(name).is_none(),
        "a deleted key must disappear from the scan too"
    );

    // 3. Restore.
    let restored = backup::restore(&directory).expect("reg import");
    assert_eq!(restored, 1);
    assert!(write::exists(&target), "the key must be back");

    // 4. Everything must be identical, values and all -- a restore that
    //    recreates the key but loses the command line is worse than none.
    let entry = find_entry(name).expect("restored key must be scannable again");
    assert_eq!(entry.display_name, DISPLAY);
    assert_eq!(entry.icon_ref.as_deref(), Some(ICON));
    assert!(entry.extended);
    match entry.kind {
        EntryKind::Verb { command, .. } => assert_eq!(command.as_deref(), Some(COMMAND)),
        other => panic!("expected a verb, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn exporting_a_missing_key_is_reported_rather_than_silently_succeeding() {
    // Counts backup directories, so it must not run while another test is
    // creating and removing one.
    let _guard = serialized();

    let target = RegTarget {
        scope: Scope::User,
        relative: r"Directory\shell\ctxmenu selftest does not exist".into(),
    };

    // Nothing exportable at all must fail loudly: a token handed out here
    // would authorise a delete with no way back.
    let before = backup::list().expect("listable").len();
    assert!(backup::export("selftest_missing", std::slice::from_ref(&target)).is_err());
    assert_eq!(
        backup::list().expect("listable").len(),
        before,
        "a failed export must not leave an empty backup behind"
    );
}
