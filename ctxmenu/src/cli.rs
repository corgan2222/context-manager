//! Command line front end.
//!
//! Hand-rolled rather than `clap`: the argument surface is tiny, `main` has to
//! inspect `argv` before starting the GUI anyway (the elevated job mode from
//! ToDo 13.2), and the release binary has a 15 MB budget to keep.
//!
//! This is a diagnostic tool for development. The shipping user interface is
//! the GUI, which gets its bilingual strings in milestone 5.

use anyhow::{Context as _, Result, bail};

use crate::model::{Category, ContextEntry, EntryKind, ScanProgress, Scope};
use crate::registry::scan::{self, ScanOptions};

pub enum Command {
    Scan(ScanArgs),
    Smoke,
    Help,
}

pub struct ScanArgs {
    pub options: ScanOptions,
    pub json: bool,
    pub quiet: bool,
}

pub const HELP: &str = "\
ctxmenu — Windows Context Menu Manager

Verwendung / Usage:
  ctxmenu scan [Optionen]   Einträge auflisten / list context menu entries
  ctxmenu --smoke           Smoke-Test-Fenster / open the smoke test window
  ctxmenu --help            Diese Hilfe / this help

Optionen / Options:
  --category <name>   Nur eine Kategorie / single category only:
                      allfiles, allfilesystemobjects, directory,
                      directorybackground, folder, desktopbackground, drive
  --scope <name>      user | machine | machine32 | all
                      (Vorgabe / default: all)
  --json              Ausgabe als JSON / emit JSON on stdout
  --quiet             Kein Fortschritt / suppress progress output
";

pub fn parse(args: impl Iterator<Item = String>) -> Result<Command> {
    let args: Vec<String> = args.collect();

    if args.is_empty() {
        return Ok(Command::Help);
    }

    match args[0].as_str() {
        "--help" | "-h" | "help" => return Ok(Command::Help),
        "--smoke" => return Ok(Command::Smoke),
        "scan" => {}
        other => bail!("Unbekannter Befehl / unknown command: {other}\n\n{HELP}"),
    }

    let mut options = ScanOptions::default();
    let mut json = false;
    let mut quiet = false;
    let mut rest = args[1..].iter();

    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--quiet" => quiet = true,
            "--category" => {
                let value = rest
                    .next()
                    .context("--category erwartet einen Namen / expects a name")?;
                let category = Category::from_slug(value)
                    .with_context(|| format!("Unbekannte Kategorie / unknown category: {value}"))?;
                options.categories = Some(vec![category]);
            }
            "--scope" => {
                let value = rest
                    .next()
                    .context("--scope erwartet einen Namen / expects a name")?;
                options.scopes =
                    if value.eq_ignore_ascii_case("all") {
                        Scope::ALL.to_vec()
                    } else {
                        vec![Scope::from_slug(value).with_context(|| {
                            format!("Unbekannter Scope / unknown scope: {value}")
                        })?]
                    };
            }
            other => bail!("Unbekannte Option / unknown option: {other}\n\n{HELP}"),
        }
    }

    Ok(Command::Scan(ScanArgs {
        options,
        json,
        quiet,
    }))
}

pub fn run_scan(args: ScanArgs) -> Result<()> {
    let started = std::time::Instant::now();

    // Progress goes to stderr so that --json keeps stdout parseable.
    let result = scan::scan(&args.options, |p: ScanProgress| {
        if !args.quiet {
            eprintln!(
                "[{:>2}/{:>2}] {:<60} {:>4} Einträge",
                p.done, p.total, p.label, p.found
            );
        }
    });

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let elapsed = started.elapsed();
    let nested = count_all(&result.entries) - result.entries.len();
    println!();
    println!(
        "{} Einträge (+{nested} in Untermenüs) in {:.2} s ({} Scopes, {})",
        result.entries.len(),
        elapsed.as_secs_f32(),
        args.options.scopes.len(),
        match &args.options.categories {
            Some(c) => c.iter().map(|c| c.slug()).collect::<Vec<_>>().join(", "),
            None => "alle Kategorien".to_string(),
        }
    );
    println!();

    println!(
        "{:<7} {:<8} {:<22} {:<34} {:<7} Befehl / CLSID",
        "Scope", "Typ", "Schlüssel", "Anzeigename", "Flags"
    );
    let rule = "-".repeat(120);
    println!("{rule}");

    for entry in &result.entries {
        print_entry(entry, 0);
    }

    print_summary(&result.entries, &args.options.scopes);
    println!(
        "MUI-Cache: {} Treffer / {} Auflösungen, blockierte CLSIDs im System: {}",
        result.stats.mui_cache_hits, result.stats.mui_cache_misses, result.stats.blocked_clsids
    );
    Ok(())
}

/// Counts entries including cascading submenu children.
fn count_all(entries: &[ContextEntry]) -> usize {
    entries
        .iter()
        .map(|e| match &e.kind {
            EntryKind::Verb { sub_commands, .. } => 1 + count_all(sub_commands),
            EntryKind::ShellEx { .. } => 1,
        })
        .sum()
}

fn print_entry(entry: &ContextEntry, indent: usize) {
    let pad = "  ".repeat(indent);
    let detail = match &entry.kind {
        EntryKind::Verb { command, .. } => command.clone().unwrap_or_else(|| "—".into()),
        EntryKind::ShellEx { clsid, .. } => clsid.clone(),
    };

    println!(
        "{:<7} {:<8} {pad}{:<22} {:<34} {:<7} {}",
        entry.scope.label(),
        entry.kind.type_label(),
        truncate(&entry.key_name, 22 - indent * 2),
        truncate(&entry.display_name, 34),
        flags(entry),
        truncate(&detail, 40),
    );

    if let EntryKind::Verb { sub_commands, .. } = &entry.kind {
        for child in sub_commands {
            print_entry(child, indent + 1);
        }
    }
}

/// Compact flag column: read-only, hidden, shift-only, forced position.
fn flags(entry: &ContextEntry) -> String {
    let mut out = String::new();
    if entry.read_only {
        out.push_str("ro ");
    }
    if entry.hidden {
        out.push_str("hid ");
    }
    if entry.extended {
        out.push_str("shift ");
    }
    if let Some(position) = &entry.position {
        out.push_str(&position.chars().take(3).collect::<String>());
    }
    out.trim_end().to_string()
}

fn print_summary(entries: &[ContextEntry], requested: &[Scope]) {
    use std::collections::BTreeMap;

    let mut per_scope: BTreeMap<&str, usize> = BTreeMap::new();
    let mut per_category: BTreeMap<String, usize> = BTreeMap::new();
    let mut shellex = 0;

    // Seed every requested scope so a scope with no hits shows up as 0 rather
    // than vanishing — "nothing found" and "not looked at" must stay
    // distinguishable.
    for scope in requested {
        per_scope.entry(scope.label()).or_default();
    }

    for entry in entries {
        *per_scope.entry(entry.scope.label()).or_default() += 1;
        *per_category.entry(entry.category.slug()).or_default() += 1;
        if matches!(entry.kind, EntryKind::ShellEx { .. }) {
            shellex += 1;
        }
    }

    println!();
    println!("Nach Scope:     {per_scope:?}");
    println!("Nach Kategorie: {per_category:?}");
    println!(
        "Davon COM-Handler: {shellex}, statische Verben: {}",
        entries.len() - shellex
    );
}

/// Truncates on character boundaries so umlauts do not split mid-encoding.
fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    let mut out: String = value.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Command> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_arguments_shows_help() {
        assert!(matches!(parse_args(&[]).unwrap(), Command::Help));
    }

    #[test]
    fn scan_defaults_to_every_scope_and_category() {
        let Command::Scan(args) = parse_args(&["scan"]).unwrap() else {
            panic!("expected a scan command");
        };
        assert_eq!(args.options.scopes.len(), 3);
        assert!(args.options.categories.is_none());
        assert!(!args.json);
    }

    #[test]
    fn category_and_scope_are_parsed() {
        let Command::Scan(args) = parse_args(&[
            "scan",
            "--category",
            "directory",
            "--scope",
            "machine32",
            "--json",
        ])
        .unwrap() else {
            panic!("expected a scan command");
        };
        assert_eq!(args.options.categories, Some(vec![Category::Directory]));
        assert_eq!(args.options.scopes, vec![Scope::Machine32]);
        assert!(args.json);
    }

    #[test]
    fn unknown_values_are_rejected_rather_than_ignored() {
        assert!(parse_args(&["scan", "--category", "nonsense"]).is_err());
        assert!(parse_args(&["scan", "--scope", "nonsense"]).is_err());
        assert!(parse_args(&["scan", "--category"]).is_err());
        assert!(parse_args(&["nonsense"]).is_err());
    }

    #[test]
    fn truncation_keeps_character_boundaries() {
        assert_eq!(truncate("kurz", 10), "kurz");
        assert_eq!(truncate("äöüäöüäöü", 4), "äöü…");
        assert_eq!(truncate("abcdef", 6), "abcdef");
    }
}
