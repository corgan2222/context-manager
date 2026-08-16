//! Group actions as a plan that is drawn up, backed up, and then executed.
//!
//! One action on a program group touches up to thirty keys across two or three
//! hives. Doing that as a loop of direct writes leaves no way to
//! say what happened when the tenth one fails, and no way to hand the elevated
//! half to another process.
//!
//! So: build a plan, take **one** backup over everything it touches, run the
//! parts that need no elevation, hand the rest to an elevated instance, and
//! collect every outcome instead of stopping at the first error. A rollback
//! mid-flight would be more complicated and less robust than a backup plus a
//! restore button.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::backup::{self, BackupToken};
use super::paths::{self, RegTarget};
use super::write;
use crate::model::Scope;

/// What to do with one entry.
///
/// Ordered from gentle to harsh, the same order the interface offers them
/// in — the delete button is deliberately not the first thing a user reaches
/// for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// `LegacyDisable=""` — hidden from the menu, trivially reversible.
    Hide,
    Show,
    /// `Extended=""` — visible only while Shift is held.
    ShiftOnly,
    AlwaysShow,
    /// `Position` = Top or Bottom, or cleared.
    ///
    /// Carried as a free string rather than an enum: values beyond those two
    /// exist in the wild (`Last`, `After` with a `PositionCompare` GUID), and
    /// an unknown one must survive a read-modify-write untouched.
    SetPosition(Option<String>),
    /// CLSID onto the machine-wide blocked list. Always needs elevation.
    Block,
    Unblock,
    /// Remove the key and everything under it.
    Delete,
}

impl Action {
    /// Can this be undone without a backup?
    ///
    /// Shown in the confirmation dialog, because "hide" and "delete" deserve
    /// very different amounts of hesitation.
    pub fn is_reversible(&self) -> bool {
        !matches!(self, Action::Delete)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Action::Hide => "Ausblenden",
            Action::Show => "Einblenden",
            Action::ShiftOnly => "Nur mit Umschalt",
            Action::AlwaysShow => "Immer zeigen",
            Action::SetPosition(_) => "Position",
            Action::Block => "Blockieren",
            Action::Unblock => "Freigeben",
            Action::Delete => "Löschen",
        }
    }
}

/// One step of a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub target: RegTarget,
    pub action: Action,
    /// Only for `Block` and `Unblock`.
    pub clsid: Option<String>,
    /// Carried along so the result dialog can name the entry rather than a
    /// registry path.
    pub display_name: String,
}

impl Operation {
    /// Does this step need an elevated process?
    ///
    /// Measured, not assumed: a HKLM key may be writable for this very
    /// process, and a HKCU key may be locked down by an ACL. Only the blocked
    /// list is decided up front, because it always lives in HKLM.
    pub fn needs_elevation(&self) -> bool {
        match self.action {
            Action::Block | Action::Unblock => !write::is_blocked_list_writable(),
            _ => !write::is_writable(&self.target),
        }
    }

    /// The registry paths this step changes, for the backup.
    pub fn backup_paths(&self) -> Vec<String> {
        match self.action {
            Action::Block | Action::Unblock => vec![paths::blocked_list_display_path()],
            _ => vec![self.target.full_path()],
        }
    }
}

/// A whole group action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// Short name of what is happening, used for the backup directory.
    pub label: String,
    pub operations: Vec<Operation>,
}

impl Plan {
    pub fn new(label: impl Into<String>, operations: Vec<Operation>) -> Self {
        Self {
            label: label.into(),
            operations,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    pub fn needs_elevation(&self) -> bool {
        self.operations.iter().any(Operation::needs_elevation)
    }

    /// Every path the plan touches, deduplicated.
    ///
    /// Deduplicated because a group action typically blocks several CLSIDs,
    /// and exporting the blocked list once per CLSID would be pointless work
    /// and a confusing manifest.
    pub fn backup_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = Vec::new();
        for operation in &self.operations {
            for path in operation.backup_paths() {
                if !paths.iter().any(|p| p.eq_ignore_ascii_case(&path)) {
                    paths.push(path);
                }
            }
        }
        paths
    }

    /// Splits into what this process can do and what needs elevation.
    ///
    /// HKCU first is not cosmetic: those steps are the ones that always
    /// succeed, so doing them before the UAC prompt means a cancelled prompt
    /// still leaves the user with partial progress they asked for.
    pub fn partition(&self) -> (Plan, Plan) {
        let (elevated, direct): (Vec<_>, Vec<_>) = self
            .operations
            .iter()
            .cloned()
            .partition(Operation::needs_elevation);

        let mut direct = direct;
        direct.sort_by_key(|o| match o.target.scope() {
            Scope::User => 0,
            Scope::Machine => 1,
            Scope::Machine32 => 2,
        });

        (
            Plan::new(self.label.clone(), direct),
            Plan::new(self.label.clone(), elevated),
        )
    }
}

/// What one step did.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResult {
    pub display_name: String,
    pub registry_path: String,
    pub action: Action,
    /// `None` on success, the message otherwise.
    pub error: Option<String>,
}

impl OperationResult {
    pub fn succeeded(&self) -> bool {
        self.error.is_none()
    }
}

/// The outcome of a whole plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// Every backup this action produced, in the order they were taken.
    ///
    /// A list, not one directory. A split plan makes two backups — one here,
    /// one in the elevated child — and keeping only the first threw away the
    /// one the HKLM changes hang on: the changes that need elevation, and the
    /// only ones that apply to every account on the machine.
    ///
    /// Reads both shapes so a result file written by an older build still
    /// loads; see [`one_or_many`].
    #[serde(default, alias = "backup_directory", deserialize_with = "one_or_many")]
    pub backup_directories: Vec<String>,
    pub results: Vec<OperationResult>,
}

/// Accepts the single value older result files carry as well as the list this
/// version writes.
///
/// A report is written by one process and read by another: the elevated child
/// leaves a result file for the parent, and a job left over from an older
/// build must not turn into a load error where the only symptom would be "the
/// elevated run left no report".
fn one_or_many<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }

    Ok(match Option::<OneOrMany>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(OneOrMany::One(single)) => vec![single],
        Some(OneOrMany::Many(list)) => list,
    })
}

impl Report {
    pub fn succeeded(&self) -> usize {
        self.results.iter().filter(|r| r.succeeded()).count()
    }

    pub fn failed(&self) -> usize {
        self.results.len() - self.succeeded()
    }

    pub fn merge(&mut self, other: Report) {
        for directory in other.backup_directories {
            if !self.backup_directories.contains(&directory) {
                self.backup_directories.push(directory);
            }
        }
        self.results.extend(other.results);
    }
}

/// Runs a plan whose steps this process can already perform.
///
/// Takes the backup itself, once, over everything the plan touches. Every step
/// is attempted even if an earlier one failed: stopping at the first error
/// would leave the user with a half-applied change and no report of which half.
pub fn execute(plan: &Plan) -> Result<Report> {
    if plan.is_empty() {
        return Ok(Report {
            backup_directories: Vec::new(),
            results: Vec::new(),
        });
    }

    let paths = plan.backup_paths();
    let token = backup::export(&plan.label, &paths)?;

    let mut results = Vec::with_capacity(plan.operations.len());
    for operation in &plan.operations {
        let outcome = apply(operation, &token);
        results.push(OperationResult {
            display_name: operation.display_name.clone(),
            registry_path: operation.target.full_path(),
            action: operation.action.clone(),
            error: outcome.err().map(|e| format!("{e:#}")),
        });
    }

    Ok(Report {
        backup_directories: vec![token.directory().display().to_string()],
        results,
    })
}

fn apply(operation: &Operation, token: &BackupToken) -> Result<()> {
    match &operation.action {
        Action::Hide => write::set_flag(&operation.target, "LegacyDisable", token),
        Action::Show => write::clear_flag(&operation.target, "LegacyDisable", token),
        Action::ShiftOnly => write::set_flag(&operation.target, "Extended", token),
        Action::AlwaysShow => write::clear_flag(&operation.target, "Extended", token),
        Action::SetPosition(value) => {
            write::set_position(&operation.target, value.as_deref(), token)
        }
        Action::Block => match &operation.clsid {
            Some(clsid) => write::block_clsid(clsid, token),
            None => anyhow::bail!("\x1eBlockieren ohne CLSID\x1fblock without a CLSID\x1d"),
        },
        Action::Unblock => match &operation.clsid {
            Some(clsid) => write::unblock_clsid(clsid, token),
            None => anyhow::bail!("\x1eFreigeben ohne CLSID\x1funblock without a CLSID\x1d"),
        },
        Action::Delete => {
            write::delete_tree(&operation.target, token)?;
            // The key is gone, so the record of having created it must go
            // too. Best effort: the deletion already happened and succeeded,
            // and a failure to tidy the bookkeeping is not worth reporting as
            // a failed deletion.
            let _ = super::create::forget_target(&operation.target);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation(relative: &str, scope: Scope, action: Action) -> Operation {
        Operation {
            target: RegTarget::below_classes(scope, relative).expect("a test path names an entry"),
            action,
            clsid: None,
            display_name: relative.into(),
        }
    }

    #[test]
    fn the_backup_covers_every_touched_path_exactly_once() {
        let plan = Plan::new(
            "test",
            vec![
                operation(r"Directory\shell\a", Scope::User, Action::Hide),
                operation(r"Directory\shell\b", Scope::User, Action::Hide),
                Operation {
                    clsid: Some("{1}".into()),
                    ..operation(r"Directory\shell\c", Scope::Machine, Action::Block)
                },
                Operation {
                    clsid: Some("{2}".into()),
                    ..operation(r"Directory\shell\d", Scope::Machine, Action::Block)
                },
            ],
        );

        let paths = plan.backup_paths();
        // Two entry keys plus the blocked list once, not twice.
        assert_eq!(paths.len(), 3, "got {paths:?}");
        assert!(
            paths
                .iter()
                .filter(|p| p.contains("Shell Extensions"))
                .count()
                == 1
        );
    }

    #[test]
    fn an_empty_plan_does_nothing_rather_than_making_a_backup() {
        let report = execute(&Plan::new("leer", Vec::new())).expect("empty plan is fine");
        assert!(report.results.is_empty());
        assert!(report.backup_directories.is_empty());
    }

    #[test]
    fn only_delete_is_irreversible() {
        assert!(Action::Hide.is_reversible());
        assert!(Action::ShiftOnly.is_reversible());
        assert!(Action::Block.is_reversible());
        assert!(Action::SetPosition(None).is_reversible());
        assert!(!Action::Delete.is_reversible());
    }

    #[test]
    fn the_direct_half_runs_hkcu_before_hklm() {
        // A cancelled UAC prompt should still leave the user with the changes
        // that never needed it.
        let plan = Plan::new(
            "test",
            vec![
                operation(r"Directory\shell\m", Scope::Machine, Action::Hide),
                operation(r"Directory\shell\u", Scope::User, Action::Hide),
            ],
        );
        let (direct, _) = plan.partition();

        let scopes: Vec<Scope> = direct.operations.iter().map(|o| o.target.scope()).collect();
        for pair in scopes.windows(2) {
            assert!(pair[0] <= pair[1], "HKCU must come first, got {scopes:?}");
        }
    }

    #[test]
    fn a_plan_survives_the_trip_through_json() {
        // The elevated half travels to another process as a file, so this is
        // not decoration: a field that fails to round-trip is a silently
        // dropped operation.
        let plan = Plan::new(
            "gruppe",
            vec![
                Operation {
                    clsid: Some("{23170F69-40C1-278A-1000-000100020000}".into()),
                    ..operation(
                        r"Directory\shellex\ContextMenuHandlers\7-Zip",
                        Scope::Machine,
                        Action::Block,
                    )
                },
                operation(
                    r"Directory\shell\x",
                    Scope::Machine32,
                    Action::SetPosition(Some("Top".into())),
                ),
            ],
        );

        let json = serde_json::to_string(&plan).expect("serialisable");
        let back: Plan = serde_json::from_str(&json).expect("deserialisable");

        assert_eq!(back.label, plan.label);
        assert_eq!(back.operations, plan.operations);
    }

    fn result(name: &str, error: Option<&str>) -> OperationResult {
        OperationResult {
            display_name: name.into(),
            registry_path: format!("HKCU\\{name}"),
            action: Action::Hide,
            error: error.map(str::to_string),
        }
    }

    #[test]
    fn a_report_counts_successes_and_failures() {
        let report = Report {
            backup_directories: vec!["dir".into()],
            results: vec![result("a", None), result("b", Some("kaputt"))],
        };
        assert_eq!(report.succeeded(), 1);
        assert_eq!(report.failed(), 1);
    }

    #[test]
    fn merging_keeps_both_backup_directories() {
        // It used to keep only the first, and the elevated half is the one
        // that comes second: the HKLM changes were made, their backup was on
        // disk, and the restore button offered the other directory -- the one
        // that could not bring them back.
        let mut report = Report {
            backup_directories: vec!["direkt".into()],
            results: vec![result("a", None)],
        };

        report.merge(Report {
            backup_directories: vec!["erhoeht".into()],
            results: vec![result("b", None)],
        });

        assert_eq!(report.results.len(), 2);
        assert_eq!(report.backup_directories, vec!["direkt", "erhoeht"]);

        // Merging the same directory twice must not offer it twice.
        report.merge(Report {
            backup_directories: vec!["erhoeht".into()],
            results: Vec::new(),
        });
        assert_eq!(report.backup_directories.len(), 2);
    }

    #[test]
    fn a_result_file_from_an_older_build_still_loads() {
        // The child writes this file and the parent reads it. A single value
        // is what every build before the list wrote, and a leftover job must
        // not come back as "the elevated run left no report".
        let single: Report = serde_json::from_str(r#"{"backup_directory":"C:\\alt","results":[]}"#)
            .expect("the old shape must still load");
        assert_eq!(single.backup_directories, vec!["C:\\alt".to_string()]);

        // Both ways of saying "there was no backup".
        for raw in [
            r#"{"backup_directory":null,"results":[]}"#,
            r#"{"results":[]}"#,
        ] {
            let report: Report = serde_json::from_str(raw).expect("still loads");
            assert!(report.backup_directories.is_empty(), "{raw}");
        }

        // And the shape this version writes, round trip.
        let written = Report {
            backup_directories: vec!["A".into(), "B".into()],
            results: vec![result("a", None)],
        };
        let json = serde_json::to_string(&written).expect("serialisable");
        let back: Report = serde_json::from_str(&json).expect("deserialisable");
        assert_eq!(back.backup_directories, written.backup_directories);
        assert_eq!(back.results.len(), 1);
    }
}
