//! Milestone 9, the half that needs no elevation.
//!
//! A group action touches many keys at once, and the promise is that it backs
//! them all up first, reports every single outcome, and can be undone from
//! that backup. All fixtures live under `HKCU\SOFTWARE\Classes` in a throwaway
//! class of this tool's own, so nothing a user would recognise is touched and
//! no VM is required. The HKLM half is exercised in the test VM instead.

use ctxmenu::model::Scope;
use ctxmenu::registry::paths::RegTarget;
use ctxmenu::registry::plan::{Action, Operation, Plan, execute};
use ctxmenu::registry::{backup, write};
use windows_registry::CURRENT_USER;

/// Each test gets its OWN throwaway class.
///
/// Two reasons. One: the scanner tests enumerate `Directory\shell` in
/// parallel, so nothing of this sort belongs there. Two, learned the hard
/// way: a single shared class made `Fixture::drop` delete the fixtures of
/// every test running beside it, which looked like a backup failure.
struct Fixture {
    class: String,
    targets: Vec<RegTarget>,
}

impl Fixture {
    fn create(class: &str, names: &[&str]) -> Self {
        let class = format!("ctxmenu_selftest_plan_{class}");
        let targets: Vec<RegTarget> = names
            .iter()
            .map(|name| RegTarget {
                scope: Scope::User,
                relative: format!(r"{class}\shell\{name}"),
            })
            .collect();

        for target in &targets {
            let key = CURRENT_USER
                .create(target.key_path())
                .expect("HKCU is writable without elevation");
            key.set_string("", "Selbsttest").expect("default value");
        }

        Self { class, targets }
    }

    fn plan(&self, action: Action) -> Plan {
        Plan::new(
            "selftest_plan",
            self.targets
                .iter()
                .map(|target| Operation {
                    target: target.clone(),
                    action: action.clone(),
                    clsid: None,
                    display_name: target.relative.clone(),
                })
                .collect(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = CURRENT_USER.remove_tree(format!(r"SOFTWARE\Classes\{}", self.class));
    }
}

fn flag_present(target: &RegTarget, name: &str) -> bool {
    CURRENT_USER
        .open(target.key_path())
        .and_then(|key| key.get_type(name))
        .is_ok()
}

#[test]
fn hiding_a_group_sets_the_flag_on_every_entry_and_can_be_undone() {
    let fixture = Fixture::create("hide", &["a", "b", "c", "d", "e"]);

    let report = execute(&fixture.plan(Action::Hide)).expect("plan runs");
    assert_eq!(report.succeeded(), 5, "every entry must be reported");
    assert_eq!(report.failed(), 0);
    assert!(report.backup_directory.is_some(), "a backup is mandatory");

    for target in &fixture.targets {
        assert!(
            flag_present(target, "LegacyDisable"),
            "{} was not hidden",
            target.full_path()
        );
    }

    // And back again — the whole point of offering this before delete.
    let report = execute(&fixture.plan(Action::Show)).expect("plan runs");
    assert_eq!(report.succeeded(), 5);
    for target in &fixture.targets {
        assert!(!flag_present(target, "LegacyDisable"));
    }

    if let Some(directory) = report.backup_directory {
        let _ = std::fs::remove_dir_all(directory);
    }
}

#[test]
fn one_backup_covers_the_whole_group_rather_than_one_per_entry() {
    let fixture = Fixture::create("backup", &["x", "y", "z"]);

    let report = execute(&fixture.plan(Action::ShiftOnly)).expect("plan runs");
    let directory = report.backup_directory.expect("backup directory");
    let path = std::path::Path::new(&directory);

    // Checked on the backup itself rather than by counting directories in
    // LOCALAPPDATA: that count is global state which every other test running
    // in parallel also changes, and an assertion over it fails for reasons
    // that have nothing to do with the property being tested.
    let manifest = backup::read_manifest(path).expect("manifest.json");
    assert_eq!(manifest.entries.len(), 3, "all three keys must be in it");
    assert!(manifest.missing.is_empty());

    let reg_files = std::fs::read_dir(path)
        .expect("readable")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "reg"))
        .count();
    assert_eq!(reg_files, 3, "one .reg per key, all in one directory");

    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn a_failing_step_does_not_stop_the_others() {
    // Four real keys and one that does not exist. The missing one must fail
    // and the rest must still be applied — stopping at the first error would
    // leave a half-applied change with no report of which half.
    let fixture = Fixture::create("partial", &["p", "q", "r", "s"]);

    let mut plan = fixture.plan(Action::Hide);
    plan.operations.insert(
        2,
        Operation {
            target: RegTarget {
                scope: Scope::User,
                relative: format!(r"{}\shell\gibt_es_nicht", fixture.class),
            },
            action: Action::Hide,
            clsid: None,
            display_name: "fehlt".into(),
        },
    );

    let report = execute(&plan).expect("plan runs despite a bad step");
    assert_eq!(report.results.len(), 5, "every step must be reported");
    assert_eq!(report.succeeded(), 4);
    assert_eq!(report.failed(), 1);

    for target in &fixture.targets {
        assert!(
            flag_present(target, "LegacyDisable"),
            "{} was skipped because of an unrelated failure",
            target.full_path()
        );
    }

    if let Some(directory) = report.backup_directory {
        let _ = std::fs::remove_dir_all(directory);
    }
}

#[test]
fn deleting_a_group_removes_every_key_and_the_backup_brings_them_back() {
    let fixture = Fixture::create("delete", &["one", "two", "three"]);

    let report = execute(&fixture.plan(Action::Delete)).expect("plan runs");
    assert_eq!(report.succeeded(), 3);
    for target in &fixture.targets {
        assert!(!write::exists(target), "{} survived", target.full_path());
    }

    let directory = report.backup_directory.expect("backup directory");
    backup::restore(std::path::Path::new(&directory)).expect("reg import");

    for target in &fixture.targets {
        assert!(
            write::exists(target),
            "{} did not come back",
            target.full_path()
        );
    }

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn an_hkcu_only_plan_needs_no_elevation() {
    let fixture = Fixture::create("elev", &["k"]);
    let plan = fixture.plan(Action::Hide);

    assert!(
        !plan.needs_elevation(),
        "keys this process just created must be writable without elevation"
    );

    let (direct, elevated) = plan.partition();
    assert_eq!(direct.operations.len(), 1);
    assert!(elevated.is_empty());
}
