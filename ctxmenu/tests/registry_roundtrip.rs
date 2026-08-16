//! End-to-end proof for milestone 3: create, scan, back up, delete, restore.
//!
//! Everything happens under `HKCU\SOFTWARE\Classes\Directory\shell\` with key
//! names that are unmistakably this tool's own. HKCU needs no elevation and
//! the fixtures remove themselves, so no VM is required — the rule that
//! registry tests belong in a VM covers HKLM tests, which none of these are.
//!
//! The fixture names carry the awkward cases on purpose: spaces, umlauts and
//! an `&`, which becomes a menu accelerator and is a classic quoting trap on
//! the way through `reg.exe`.

use std::path::{Path, PathBuf};

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

/// The same name as the shell would draw it.
///
/// The `&` marks the following character — here a space — and is not shown, so
/// two spaces are left where three characters used to be. Unattractive, and
/// exactly the point: this is what the menu really says, and a tool that
/// repeats the raw value would hide the mistake instead of showing it. The
/// unchanged original stays in `raw_display`.
const DISPLAY_IN_MENU: &str = "Selbsttest mit Leerzeichen  Ümläut";
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
        let target = RegTarget::below_classes(Scope::User, &format!(r"Directory\shell\{name}"))
            .expect("a fixture path names an entry");

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

/// Removes every backup directory a test produced, even when a `Drop` runs
/// because the test panicked rather than finished.
///
/// `backup::export_targets` writes into the user's own
/// `%LOCALAPPDATA%\ctxmenu\backups`, and an assertion standing between that
/// call and an end-of-test `remove_dir_all` could unwind straight past it,
/// leaving the directory there for the backup tab to offer as if it were the
/// user's own. Same fix as [`Fixture`], for the directory tree beside the
/// registry keys instead of the keys themselves.
struct BackupGuard(Vec<PathBuf>);

impl BackupGuard {
    fn none() -> Self {
        Self(Vec::new())
    }

    fn track(&mut self, directory: impl AsRef<Path>) {
        self.0.push(directory.as_ref().into());
    }
}

impl Drop for BackupGuard {
    fn drop(&mut self) {
        for directory in &self.0 {
            remove_dir_all_with_retry(directory);
        }
    }
}

/// Retries a directory removal for the same reason `backup::export` retries
/// creating one -- and for longer, because deleting is contested harder.
///
/// Measured on a machine doing other work at the same time: a `remove_dir_all`
/// right after `export_targets()` can meet `ERROR_SHARING_VIOLATION` while a
/// search indexer or endpoint security scanner still has a freshly written
/// `.reg` file open, well past the roughly two-second budget that is enough
/// for `backup::export`'s own retries on the way in. Ten seconds covers the
/// ordinary case for free -- a successful first attempt returns immediately,
/// nothing here ever slows down a clean run. It cannot promise the directory
/// is gone by the time this function returns: on a machine busy enough, the
/// same contention can outlast any bounded retry a test's `Drop` could afford
/// without turning a rare failure into a multi-minute one. What it does
/// promise, and what the bug this fixes needed, is that removal is *attempted*
/// on the panic path exactly as it always was on the success path, instead of
/// never at all -- and every attempt made here is one the pre-`Drop` code
/// never got to make.
fn remove_dir_all_with_retry(directory: &Path) {
    for attempt in 0u32..20 {
        match std::fs::remove_dir_all(directory) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) if attempt < 19 => {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(_) => {}
        }
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

    // Two separate claims: the raw value survived `reg.exe` and the registry
    // API byte for byte, and the name shown on screen is the one the menu
    // draws. Checking only the first would have let the accelerator through to
    // the interface; checking only the second would no longer prove the
    // quoting.
    assert_eq!(entry.raw_display.as_deref(), Some(DISPLAY));
    assert_eq!(entry.display_name, DISPLAY_IN_MENU);
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
    let token = backup::export_targets("selftest", std::slice::from_ref(&target))
        .expect("export of an existing key");
    let directory = token.directory().to_path_buf();
    let mut backups = BackupGuard::none();
    backups.track(&directory);

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
    assert_eq!(restored.restored, 1, "{:?}", restored.failures);
    assert_eq!(restored.removed, 0, "the key existed when it was backed up");
    assert!(write::exists(&target), "the key must be back");

    // 4. Everything must be identical, values and all -- a restore that
    //    recreates the key but loses the command line is worse than none.
    let entry = find_entry(name).expect("restored key must be scannable again");
    // The raw value is the one that had to survive the round trip through
    // `reg.exe export` and `reg.exe import`; the `&` is in there precisely
    // because it is a quoting trap on that route.
    assert_eq!(entry.raw_display.as_deref(), Some(DISPLAY));
    assert_eq!(entry.display_name, DISPLAY_IN_MENU);
    assert_eq!(entry.icon_ref.as_deref(), Some(ICON));
    assert!(entry.extended);
    match entry.kind {
        EntryKind::Verb { command, .. } => assert_eq!(command.as_deref(), Some(COMMAND)),
        other => panic!("expected a verb, got {other:?}"),
    }
}

/// What the editor writes as a submenu, read back by the scanner.
///
/// Both halves in one test on purpose. The form's promise is not "a key with
/// these three values" but "an entry that opens onto its children in the right
/// order", and only the scanner can say whether that arrived. It also settles
/// the one question the shape of the keys cannot: the registry hands subkeys
/// back alphabetically, so a submenu whose children are meant to keep the
/// order they were typed in has to carry that order in their names.
#[test]
fn a_submenu_written_by_the_editor_comes_back_as_a_cascading_entry() {
    use ctxmenu::registry::create::{self, NewChild, NewEntry};

    let _guard = serialized();
    let name = "ctxmenu selftest submenu";

    // Deliberately in the order that is *not* alphabetical: without the
    // numbered key names the menu would show Anton first.
    let children: Vec<NewChild> = ["Zebra", "Anton"]
        .iter()
        .enumerate()
        .map(|(index, display)| NewChild {
            key_name: create::suggest_child_key_name(index, display),
            display_name: (*display).into(),
            command: format!(r#""C:\Windows\system32\cmd.exe" /c echo {display}"#),
            icon: None,
        })
        .collect();

    let entry = NewEntry {
        category: Category::Directory,
        key_name: name.into(),
        display_name: "Selbsttest-Untermenü".into(),
        command: String::new(),
        icon: None,
        position: None,
        extended: false,
        children: children.clone(),
    };

    let made = create::create(&entry).expect("HKCU takes an entry without elevation");
    assert!(
        made.note.is_none(),
        "nothing should have gone wrong beside the entry: {:?}",
        made.note
    );
    let target = made.target;
    // Same reason as `Fixture`: an assertion below unwinds, and a stray
    // submenu in the user's real folder menu is a rude way to fail.
    struct Cleanup(RegTarget);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = CURRENT_USER.remove_tree(self.0.key_path());
            let _ = ctxmenu::registry::create::forget_target(&self.0);
        }
    }
    let cleanup = Cleanup(target.clone());

    let found = find_entry(name).expect("the submenu must show up in a Directory scan");
    let EntryKind::Verb {
        command,
        sub_commands,
    } = &found.kind
    else {
        panic!("expected a verb");
    };

    assert_eq!(found.display_name, "Selbsttest-Untermenü");
    assert!(
        command.is_none(),
        "a submenu runs nothing itself, so it has no command line"
    );
    assert_eq!(sub_commands.len(), 2, "got {sub_commands:?}");

    // The order the user chose, not the one the registry would produce.
    assert_eq!(sub_commands[0].display_name, "Zebra");
    assert_eq!(sub_commands[1].display_name, "Anton");

    for (child, written) in sub_commands.iter().zip(&children) {
        let EntryKind::Verb { command, .. } = &child.kind else {
            panic!("expected a verb");
        };
        assert_eq!(command.as_deref(), Some(written.command.as_str()));
        // A child is addressable in its own right — that is what makes it
        // selectable and deletable in the window.
        assert_eq!(
            child.registry_path,
            format!(r"{}\shell\{}", target.full_path(), written.key_name)
        );
        assert!(RegTarget::parse(&child.registry_path).is_ok());
    }

    drop(cleanup);
    assert!(
        !write::exists(&target),
        "the fixture cleans up after itself"
    );
}

/// The second kind of cascading menu: `SubCommands` names verbs that live in
/// the CommandStore instead of below the entry itself.
///
/// Built here rather than found: measured on this machine, 15 entries carry a
/// `SubCommands` value and every one of them is empty — the marker form, which
/// means "my children are in my own `shell` subkey". So the resolving path has
/// no natural specimen, and the honest way to prove it is to make one.
#[test]
fn a_subcommands_list_pulls_its_children_out_of_the_command_store() {
    let _guard = serialized();

    // Whatever this Windows happens to stock, rather than a hard-coded verb
    // name: the store differs between installations.
    let store: Vec<ctxmenu::model::ContextEntry> = scan(
        &ScanOptions {
            categories: Some(vec![Category::CommandStore]),
            ..ScanOptions::default()
        },
        |_| {},
    )
    .entries;
    let Some(borrowed) = store.first() else {
        // A machine with an empty CommandStore proves nothing either way.
        return;
    };
    let verb = borrowed.key_name.clone();

    let name = "ctxmenu selftest subcommands";
    let fixture = Fixture::create(name);
    CURRENT_USER
        .create(fixture.target.key_path())
        .expect("fixture key")
        .set_string("SubCommands", &verb)
        .expect("verb list");

    let entry = find_entry(name).expect("the fixture must show up in a Directory scan");
    let EntryKind::Verb { sub_commands, .. } = &entry.kind else {
        panic!("expected a verb");
    };

    assert_eq!(sub_commands.len(), 1, "one name, one child");
    let child = &sub_commands[0];
    assert_eq!(child.key_name, verb);
    assert!(
        child.read_only,
        "a verb belonging to Windows must not look editable"
    );
    // The child keeps the path it really has. Reporting it below the parent
    // would name a key that does not exist — and hand the delete path a
    // location it must never touch.
    assert_eq!(child.registry_path, borrowed.registry_path);
    assert!(
        ctxmenu::registry::paths::RegTarget::parse(&child.registry_path).is_err(),
        "a CommandStore path must not be expressible as a target"
    );
}

#[test]
fn an_unknown_name_in_a_subcommands_list_is_left_out_rather_than_faked() {
    let _guard = serialized();

    let name = "ctxmenu selftest unknown subcommand";
    let fixture = Fixture::create(name);
    CURRENT_USER
        .create(fixture.target.key_path())
        .expect("fixture key")
        .set_string("SubCommands", "Ctxmenu.GibtEsNicht;;  ")
        .expect("verb list");

    let entry = find_entry(name).expect("the fixture must show up in a Directory scan");
    let EntryKind::Verb { sub_commands, .. } = &entry.kind else {
        panic!("expected a verb");
    };
    // Windows leaves an unresolvable name out of the menu; a row for it would
    // report a menu item that is not there. Empty segments likewise.
    assert!(sub_commands.is_empty(), "got {sub_commands:?}");
}

#[test]
fn exporting_a_missing_key_records_an_empty_state_without_licensing_a_delete() {
    // Counts backup directories, so it must not run while another test is
    // creating and removing one.
    let _guard = serialized();

    let target = RegTarget::below_classes(
        Scope::User,
        r"Directory\shell\ctxmenu selftest does not exist",
    )
    .expect("a fixture path names an entry");

    // A key that is not there is an empty starting state, not a failed
    // export. Until this changed, a plan of nothing but `Block` steps could
    // not run at all on a machine where nothing had ever been blocked: Windows
    // ships no blocked list, so its backup had nothing to write.
    let before = backup::list().expect("listable").len();
    let token = backup::export_targets("selftest_missing", std::slice::from_ref(&target))
        .expect("an empty starting state is a state");
    // A safety net, not the primary cleanup below: an assertion failing
    // between here and the manual `remove_dir_all` must still not leave the
    // directory behind for good.
    let mut backups = BackupGuard::none();
    backups.track(token.directory());

    let manifest = backup::read_manifest(token.directory()).expect("manifest.json");
    assert!(manifest.entries.is_empty(), "there was nothing to export");
    assert_eq!(manifest.absent, vec![target.full_path()]);

    // And the promise the old failure was protecting stands where it always
    // stood: a delete needs the contents of the key on disk, and "there was
    // nothing here" is not that.
    assert!(!token.covers(&target));
    assert!(
        write::delete_tree(&target, &token).is_err(),
        "an absence must not authorise a delete"
    );

    let _ = std::fs::remove_dir_all(token.directory());
    assert_eq!(
        backup::list().expect("listable").len(),
        before,
        "the test cleans up after itself"
    );
}
