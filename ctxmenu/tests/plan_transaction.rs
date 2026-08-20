//! Milestone 9, the half that needs no elevation.
//!
//! A group action touches many keys at once, and the promise is that it backs
//! them all up first, reports every single outcome, and can be undone from
//! that backup. All fixtures live under `HKCU\SOFTWARE\Classes` in a throwaway
//! class of this tool's own, so nothing a user would recognise is touched and
//! no VM is required. The HKLM half is exercised in the test VM instead.

use std::path::{Path, PathBuf};

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
            .map(|name| {
                RegTarget::below_classes(Scope::User, &format!(r"{class}\shell\{name}"))
                    .expect("a fixture path names an entry")
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
                    display_name: target.relative().to_string(),
                    packaged: false,
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

/// Removes every backup directory a test produced, even when a `Drop` runs
/// because the test panicked rather than finished.
///
/// Not cosmetic: every run of this file writes into
/// `%LOCALAPPDATA%\ctxmenu\backups`, and the tests once left 266 directories
/// there, all of which the backup tab of the application then offered as if
/// they were the user's. An end-of-test cleanup call fixed the ordinary case,
/// but every assertion standing between `execute()` and that call could
/// unwind straight past it. `Drop` is the same fix `Fixture` already has for
/// the registry keys, applied to the directory tree beside them.
struct BackupGuard(Vec<PathBuf>);

impl BackupGuard {
    fn none() -> Self {
        Self(Vec::new())
    }

    /// Remembers one more directory to remove.
    ///
    /// A test may call `execute` more than once -- hide, then show again --
    /// and must not lose track of the first backup while waiting for the
    /// second.
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
/// right after `execute()` can meet `ERROR_SHARING_VIOLATION` while a search
/// indexer or endpoint security scanner still has a freshly written `.reg`
/// file open, well past the roughly two-second budget that is enough for
/// `backup::export`'s own retries on the way in. Ten seconds covers the
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

/// Every failed step with its reason, plus what the backup made of it.
///
/// A bare count in an assertion says "two of three" and nothing about which one
/// or why; that turned a rare failure into a guessing game once already. The
/// manifest belongs in the same message because the usual suspect is a key that
/// never made it into the backup — a step whose backup is missing is refused,
/// and then only the refusal is visible, not the cause.
fn failures(report: &ctxmenu::registry::plan::Report) -> String {
    let mut text = String::new();

    for result in report.results.iter().filter(|r| !r.succeeded()) {
        text.push_str(&format!(
            "\n  {} -> {}",
            result.registry_path,
            result.error.as_deref().unwrap_or("?")
        ));
    }

    for directory in &report.backup_directories {
        match backup::read_manifest(std::path::Path::new(directory)) {
            Ok(manifest) => {
                text.push_str(&format!(
                    "\n  Backup {}: {} gesichert, fehlend {:?}, Meldungen {:?}",
                    directory,
                    manifest.entries.len(),
                    manifest.missing,
                    manifest.notes
                ));
            }
            Err(error) => text.push_str(&format!("\n  Backup {directory}: {error:#}")),
        }
    }

    text
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
    let mut backups = BackupGuard::none();

    let report = execute(&fixture.plan(Action::Hide)).expect("plan runs");
    report
        .backup_directories
        .iter()
        .for_each(|d| backups.track(d));
    assert_eq!(
        report.succeeded(),
        5,
        "every entry must be reported{}",
        failures(&report)
    );
    assert_eq!(report.failed(), 0);
    assert_eq!(
        report.backup_directories.len(),
        1,
        "one backup, and it is mandatory"
    );

    for target in &fixture.targets {
        assert!(
            flag_present(target, "LegacyDisable"),
            "{} was not hidden",
            target.full_path()
        );
    }

    // And back again — the whole point of offering this before delete.
    let report = execute(&fixture.plan(Action::Show)).expect("plan runs");
    report
        .backup_directories
        .iter()
        .for_each(|d| backups.track(d));
    assert_eq!(report.succeeded(), 5, "{}", failures(&report));
    for target in &fixture.targets {
        assert!(!flag_present(target, "LegacyDisable"));
    }
}

#[test]
fn one_backup_covers_the_whole_group_rather_than_one_per_entry() {
    let fixture = Fixture::create("backup", &["x", "y", "z"]);
    let mut backups = BackupGuard::none();

    let report = execute(&fixture.plan(Action::ShiftOnly)).expect("plan runs");
    let [directory] = &report.backup_directories[..] else {
        panic!("one plan, one backup, got {:?}", report.backup_directories);
    };
    backups.track(directory);
    let path = std::path::Path::new(directory);

    // Checked on the backup itself rather than by counting directories in
    // LOCALAPPDATA: that count is global state which every other test running
    // in parallel also changes, and an assertion over it fails for reasons
    // that have nothing to do with the property being tested.
    let manifest = backup::read_manifest(path).expect("manifest.json");
    assert_eq!(
        manifest.entries.len(),
        3,
        "all three keys must be in it, missing {:?}, notes {:?}",
        manifest.missing,
        manifest.notes
    );
    assert!(manifest.missing.is_empty());

    let reg_files = std::fs::read_dir(path)
        .expect("readable")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "reg"))
        .count();
    assert_eq!(reg_files, 3, "one .reg per key, all in one directory");
}

#[test]
fn a_failing_step_does_not_stop_the_others() {
    // Four real keys and one that does not exist. The missing one must fail
    // and the rest must still be applied — stopping at the first error would
    // leave a half-applied change with no report of which half.
    let fixture = Fixture::create("partial", &["p", "q", "r", "s"]);
    let mut backups = BackupGuard::none();

    let mut plan = fixture.plan(Action::Hide);
    plan.operations.insert(
        2,
        Operation {
            target: RegTarget::below_classes(
                Scope::User,
                &format!(r"{}\shell\gibt_es_nicht", fixture.class),
            )
            .expect("a fixture path names an entry"),
            action: Action::Hide,
            clsid: None,
            display_name: "fehlt".into(),
            packaged: false,
        },
    );

    let report = execute(&plan).expect("plan runs despite a bad step");
    report
        .backup_directories
        .iter()
        .for_each(|d| backups.track(d));
    assert_eq!(report.results.len(), 5, "every step must be reported");
    assert_eq!(
        report.succeeded(),
        4,
        "only the invented key may fail{}",
        failures(&report)
    );
    assert_eq!(report.failed(), 1);

    for target in &fixture.targets {
        assert!(
            flag_present(target, "LegacyDisable"),
            "{} was skipped because of an unrelated failure",
            target.full_path()
        );
    }
}

/// What the backup directory actually holds, for an assertion that failed.
///
/// "did not come back" on its own says nothing a reader can act on: it does
/// not say whether the `.reg` file was written, whether it had any content,
/// what the manifest recorded, or which of the three keys made it. This
/// failure appeared only on the GitHub runner and never on the development
/// machine, and the first two attempts at reading it were guesswork for
/// exactly that reason.
fn evidence(directory: &Path, fixture: &Fixture) -> String {
    let mut out = String::from(
        "--- backup directory ---
",
    );
    out.push_str(&format!(
        "{}
",
        directory.display()
    ));

    match std::fs::read_dir(directory) {
        Ok(entries) => {
            let mut files: Vec<_> = entries.flatten().collect();
            files.sort_by_key(std::fs::DirEntry::file_name);
            for file in files {
                let size = file.metadata().map(|m| m.len()).unwrap_or(0);
                out.push_str(&format!(
                    "  {:>7} B  {:?}
",
                    size,
                    file.file_name()
                ));

                // The manifest decides what restore even attempts, and a .reg
                // file that exported nothing still imports with exit code 0 --
                // which is how "3 restored" and a missing key can both be true.
                let name = file.file_name();
                let name = name.to_string_lossy();
                if name == "manifest.json" || name.ends_with(".reg") {
                    match std::fs::read(file.path()) {
                        Ok(bytes) => {
                            let text = if bytes.starts_with(&[0xFF, 0xFE]) {
                                let wide: Vec<u16> = bytes[2..]
                                    .chunks_exact(2)
                                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                                    .collect();
                                String::from_utf16_lossy(&wide)
                            } else {
                                String::from_utf8_lossy(&bytes).into_owned()
                            };
                            for line in text.lines() {
                                out.push_str(&format!(
                                    "      | {line}
"
                                ));
                            }
                        }
                        Err(error) => out.push_str(&format!(
                            "      | unreadable: {error}
"
                        )),
                    }
                }
            }
        }
        Err(error) => out.push_str(&format!(
            "  unreadable: {error}
"
        )),
    }

    out.push_str(
        "--- keys now ---
",
    );
    for target in &fixture.targets {
        out.push_str(&format!(
            "  {} exists={}
",
            target.full_path(),
            write::exists(target)
        ));
    }
    out
}

#[test]
fn deleting_a_group_removes_every_key_and_the_backup_brings_them_back() {
    let fixture = Fixture::create("delete", &["one", "two", "three"]);
    let mut backups = BackupGuard::none();

    let report = execute(&fixture.plan(Action::Delete)).expect("plan runs");
    report
        .backup_directories
        .iter()
        .for_each(|d| backups.track(d));
    assert_eq!(report.succeeded(), 3, "{}", failures(&report));
    for target in &fixture.targets {
        assert!(!write::exists(target), "{} survived", target.full_path());
    }

    let [directory] = &report.backup_directories[..] else {
        panic!("one plan, one backup, got {:?}", report.backup_directories);
    };
    let restored = backup::restore(std::path::Path::new(directory)).expect("reg import");
    assert_eq!(restored.restored, 3, "{:?}", restored.failures);
    assert_eq!(restored.removed, 0, "nothing was missing when it was taken");

    for target in &fixture.targets {
        assert!(
            write::exists(target),
            "{} did not come back\n{}",
            target.full_path(),
            evidence(Path::new(directory), &fixture)
        );
    }
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

/// A value name no real handler ever carries: not hex, and it says in plain
/// letters where it came from, so a run that dies mid-test leaves an
/// obviously harmless leftover.
const TEST_CLSID: &str = "{CTXMENU-SELFTEST-PACKAGED-VERB}";

/// The packaged step never writes its carrier target — the CLSID is the
/// lever. The target still has to parse, which any fixture key does.
fn packaged_operation(fixture: &Fixture, action: Action) -> Plan {
    Plan::new(
        "selftest_packaged",
        vec![Operation {
            target: fixture.targets[0].clone(),
            action,
            clsid: Some(TEST_CLSID.into()),
            display_name: "selftest packaged".into(),
            packaged: true,
        }],
    )
}

fn user_blocked_value_present() -> bool {
    CURRENT_USER
        .open(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Blocked")
        .ok()
        .is_some_and(|key| key.get_type(TEST_CLSID).is_ok())
}

/// Hiding a packaged entry is a value in the user's blocked list, and
/// showing it again removes exactly that value — the mechanism measured in
/// the Windows 11 VM on 2026-08-20, here exercised through the full plan
/// path with its mandatory backup.
#[test]
fn hiding_a_packaged_entry_blocks_its_clsid_for_this_user_and_frees_it_again() {
    let fixture = Fixture::create("packaged", &["carrier"]);
    let mut backups = BackupGuard::none();

    let plan = packaged_operation(&fixture, Action::Hide);
    assert!(
        !plan.needs_elevation(),
        "the user's own blocked list must not ask for admin"
    );

    let report = execute(&plan).expect("plan runs");
    report
        .backup_directories
        .iter()
        .for_each(|d| backups.track(d));
    assert_eq!(report.failed(), 0, "{}", failures(&report));
    assert!(
        user_blocked_value_present(),
        "the CLSID must be on the list"
    );

    // The carrier key itself must be untouched: no flag, no value.
    assert!(!flag_present(&fixture.targets[0], "LegacyDisable"));

    let report = execute(&packaged_operation(&fixture, Action::Show)).expect("plan runs");
    report
        .backup_directories
        .iter()
        .for_each(|d| backups.track(d));
    assert_eq!(report.failed(), 0, "{}", failures(&report));
    assert!(
        !user_blocked_value_present(),
        "showing again must remove the value"
    );
}

/// Delete, position and the Shift rule do not exist for a packaged entry.
/// The plan path refuses them itself — the interface greys them out, but a
/// plan can arrive from anywhere.
#[test]
fn a_packaged_entry_refuses_what_only_registry_keys_can_do() {
    let fixture = Fixture::create("packaged_refuse", &["carrier"]);
    let mut backups = BackupGuard::none();

    for action in [
        Action::Delete,
        Action::ShiftOnly,
        Action::SetPosition(Some("Top".into())),
    ] {
        let report = execute(&packaged_operation(&fixture, action)).expect("plan runs");
        report
            .backup_directories
            .iter()
            .for_each(|d| backups.track(d));
        assert_eq!(report.failed(), 1, "the step must fail, not be skipped");
    }

    // Refused loudly, and nothing happened: the carrier key is still there.
    let key = CURRENT_USER.open(fixture.targets[0].key_path());
    assert!(key.is_ok(), "the carrier key must survive every refusal");
}
