//! The entries of the new Windows 11 context menu.
//!
//! They are not registry keys under `…\shell`. Each one is an
//! `IExplorerCommand` handler that a package with identity declares in its
//! `AppxManifest.xml`, and the menu is built from those declarations. This
//! module finds them the way the shell does, but without the WinRT package
//! API — measured against `Get-AppxPackage` on 2026-08-20 in the test VM
//! (build 26200), both routes returned the same packages:
//!
//! 1. `Software\Classes\PackagedCom\Package` names every package that
//!    registered COM classes, in HKLM and HKCU.
//! 2. The repository key under `HKCU\…\AppModel\Repository\Packages` holds
//!    each package's `PackageRootFolder`.
//! 3. `AppxManifest.xml` in that folder declares the verbs: a
//!    `windows.fileExplorerContextMenus` extension names item types and
//!    verb CLSIDs, a `windows.comServer` extension maps each CLSID to the
//!    DLL behind it.
//!
//! What the manifest does *not* hold is the menu text. That is produced at
//! run time by the handler's `GetTitle`, and loading a foreign DLL to ask it
//! is not something this program does. The verb id and the package name are
//! what can be shown honestly.
//!
//! Hiding one of these entries is a different lever than for classic ones:
//! no `LegacyDisable`, but the CLSID as an empty value in the blocked list —
//! see `rules/win11-menue.md` for the measurements. This module only reads
//! which CLSIDs are blocked; writing is the plan path's job.

use std::path::{Path, PathBuf};

use serde::Serialize;
use windows_registry::{CURRENT_USER, Key, LOCAL_MACHINE};

use super::mui::MuiResolver;
use super::paths::SHELL_EXTENSIONS_BLOCKED;
use super::scan::subkey_names;
use crate::model::Scope;

/// Where the package roots are recorded, relative to `HKCU`.
///
/// This is the registry mirror of the state repository. `Get-AppxPackage`
/// answers from the same data; reading the mirror spares the WinRT
/// dependency.
const REPOSITORY_PACKAGES: &str = "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel\\Repository\\Packages";

/// Package lists live here, relative to either hive.
const PACKAGED_COM_PACKAGE: &str = "Software\\Classes\\PackagedCom\\Package";

/// Everything the new menu gets from installed packages.
#[derive(Debug, Default, Clone, Serialize)]
pub struct PackagedMenu {
    pub packages: Vec<PackagedPackage>,
    /// Candidates that had no repository entry, no root folder or no
    /// readable manifest. Not an error — most COM packages have no context
    /// menu — but worth counting, because a silently short list looks
    /// exactly like a complete one.
    pub skipped: usize,
}

/// One package that declares context menu verbs.
#[derive(Debug, Clone, Serialize)]
pub struct PackagedPackage {
    /// Full name with version and publisher hash, the registry's key name.
    pub full_name: String,
    /// Resolved display name, or the family part of the full name when the
    /// `ms-resource:` reference cannot be resolved.
    pub display_name: String,
    /// `PackageRootFolder` from the repository key.
    pub root: PathBuf,
    /// Absolute path to the package logo, when the manifest names one that
    /// exists on disk.
    pub logo: Option<PathBuf>,
    /// `User` when the user's own hive registered the package, `Machine`
    /// when only HKLM did (a provisioned package).
    pub scope: Scope,
    pub verbs: Vec<PackagedVerb>,
}

/// One verb — one entry in the new menu, and the unit the blocked list works
/// in.
#[derive(Debug, Clone, Serialize)]
pub struct PackagedVerb {
    /// The manifest's verb id, e.g. `OpenInNotepad`. Not the menu text, but
    /// the closest thing to it that exists outside the handler DLL.
    pub id: String,
    /// Normalised to braces and upper case, the spelling the blocked list
    /// uses.
    pub clsid: String,
    /// What the verb applies to: `*`, `Directory`, `Directory\Background`,
    /// or an extension like `.txt`.
    pub item_types: Vec<String>,
    /// The DLL behind the CLSID, from the `windows.comServer` extension.
    /// Relative paths are anchored at the package root.
    pub dll: Option<PathBuf>,
    /// Blocked in `HKCU` — hidden for this user, the lever this program
    /// writes.
    pub blocked_user: bool,
    /// Blocked in `HKLM` — hidden machine-wide, shown but never written.
    pub blocked_machine: bool,
}

/// Reads the full picture: packages, verbs, blocked state.
pub fn scan() -> PackagedMenu {
    let mut menu = PackagedMenu::default();
    let blocked_user = blocked_values(CURRENT_USER);
    let blocked_machine = blocked_values(LOCAL_MACHINE);
    let mut mui = MuiResolver::new();

    for (full_name, scope) in candidates() {
        let Some(root) = package_root(&full_name) else {
            menu.skipped += 1;
            continue;
        };
        // The manifest of a normal package sits at its root. A package with
        // external location keeps its *content* elsewhere — the repository
        // root points there — while the manifest stays in the WindowsApps
        // store folder. Adobe Acrobat is the case that found this: root
        // `…\Adobe\Acrobat DC`, manifest under WindowsApps, and the store
        // folder is user-readable. Measured 2026-08-20.
        let store_dir = windows_apps_dir(&full_name);
        let Some(xml) = std::fs::read_to_string(root.join("AppxManifest.xml"))
            .ok()
            .or_else(|| {
                let dir = store_dir.as_ref()?;
                std::fs::read_to_string(dir.join("AppxManifest.xml")).ok()
            })
        else {
            menu.skipped += 1;
            continue;
        };
        let Some(parsed) = parse_manifest(&xml) else {
            menu.skipped += 1;
            continue;
        };
        if parsed.verbs.is_empty() {
            // A COM package without context menu verbs — the common case.
            continue;
        }

        let verbs = parsed
            .verbs
            .into_iter()
            .map(|verb| {
                let blocked_u = blocked_user.contains(&verb.clsid);
                let blocked_m = blocked_machine.contains(&verb.clsid);
                PackagedVerb {
                    dll: verb.dll.map(|dll| anchor(&root, &dll)),
                    blocked_user: blocked_u,
                    blocked_machine: blocked_m,
                    id: verb.id,
                    clsid: verb.clsid,
                    item_types: verb.item_types,
                }
            })
            .collect();

        let display_name = display_name(&full_name, parsed.display_name.as_deref(), &mut mui);
        // Manifest paths resolve against the external content first, the
        // store folder second — the same order the shell tries.
        let logo = parsed.logo.and_then(|logo| {
            let logo = Path::new(&logo);
            [
                Some(anchor(&root, logo)),
                store_dir.map(|dir| dir.join(logo)),
            ]
            .into_iter()
            .flatten()
            .find(|candidate| candidate.exists())
        });

        menu.packages.push(PackagedPackage {
            full_name,
            display_name,
            root,
            logo,
            scope,
            verbs,
        });
    }

    menu.packages.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
    });
    menu
}

/// Package names from both `PackagedCom` lists, user hive first.
///
/// The same package is usually registered in both — installed for the user
/// *and* provisioned — so the list is deduplicated on the full name, and the
/// first (user) hit decides the scope.
fn candidates() -> Vec<(String, Scope)> {
    let mut seen = rustc_hash::FxHashSet::default();
    let mut out = Vec::new();

    for (hive, scope) in [(CURRENT_USER, Scope::User), (LOCAL_MACHINE, Scope::Machine)] {
        let Ok(key) = hive.open(PACKAGED_COM_PACKAGE) else {
            continue;
        };
        for name in subkey_names(&key) {
            // Sits between the package keys but is a counter, not a package.
            if name.eq_ignore_ascii_case("MaxInstallOrder") {
                continue;
            }
            if seen.insert(name.to_lowercase()) {
                out.push((name, scope));
            }
        }
    }
    out
}

/// The packaged verbs as scan entries, one per verb and item type.
///
/// One entry per (verb, item type) rather than one per verb: a
/// [`ContextEntry`] carries exactly one [`Category`], and the file-type view
/// attributes entries to extensions through it — the same shape a classic
/// verb takes when it is registered under every extension separately.
///
/// `registry_path` is the package's real `PackagedCom` key. It is never
/// written; it exists so the entry has a stable id, a scope and a path the
/// detail pane can show. Hiding works on the CLSID, not on this key.
pub fn entries(menu: &PackagedMenu) -> Vec<crate::model::ContextEntry> {
    use crate::model::{ContextEntry, EntryKind, stable_id};

    let mut out = Vec::new();
    for package in &menu.packages {
        let path = format!(
            "{}\\{}\\{}\\{}",
            package.scope.hive(),
            package.scope.classes_path(),
            PACKAGED_COM_PACKAGE.trim_start_matches("Software\\Classes\\"),
            package.full_name
        );
        for verb in &package.verbs {
            for item_type in &verb.item_types {
                let Some(category) = category_for(item_type) else {
                    continue;
                };
                out.push(ContextEntry {
                    // The item type is part of the id: the same verb appears
                    // once per item type, and two rows must never share one.
                    id: stable_id(package.scope, &format!("{path}|{}|{item_type}", verb.clsid)),
                    key_name: verb.id.clone(),
                    // The honest approximation of the menu text — the real
                    // one lives in the handler's GetTitle (module comment).
                    display_name: verb.id.clone(),
                    raw_display: None,
                    icon_ref: package
                        .logo
                        .as_ref()
                        .map(|logo| logo.to_string_lossy().into_owned()),
                    position: None,
                    extended: false,
                    // The per-user block is exactly what "hidden" means for
                    // this kind, measured 2026-08-20: it takes the entry out
                    // of the menu on the next open, no Explorer restart.
                    hidden: verb.blocked_user,
                    applies_to: Some(item_type.clone()),
                    kind: EntryKind::PackagedVerb {
                        clsid: verb.clsid.clone(),
                        package: package.full_name.clone(),
                        package_name: package.display_name.clone(),
                        dll: verb
                            .dll
                            .as_ref()
                            .map(|dll| dll.to_string_lossy().into_owned()),
                        blocked_machine: verb.blocked_machine,
                    },
                    scope: package.scope,
                    category,
                    registry_path: path.clone(),
                    // Hiding goes to the HKCU blocked list, which every user
                    // may write; nothing else can be done to this entry.
                    read_only: false,
                    program_key: verb
                        .dll
                        .as_ref()
                        .map(|dll| dll.to_string_lossy().into_owned()),
                });
            }
        }
    }
    out
}

/// Where an item type puts the entry, in the vocabulary of the classic scan.
///
/// An extension maps to [`Category::ExtAssoc`], which is what
/// `attribute_to_file_types` matches on — the file-type view picks the entry
/// up without knowing it came from a package. Unknown item types are dropped
/// rather than guessed into a category they do not belong to.
fn category_for(item_type: &str) -> Option<crate::model::Category> {
    use crate::model::Category;
    match item_type {
        "*" => Some(Category::AllFiles),
        "Directory" => Some(Category::Directory),
        "Directory\\Background" => Some(Category::DirectoryBackground),
        "Drive" => Some(Category::Drive),
        ext if ext.starts_with('.') => Some(Category::ExtAssoc(ext.to_lowercase())),
        _ => None,
    }
}

/// The package's folder in the WindowsApps store, where a sparse package
/// keeps its manifest.
///
/// Derived rather than read from the registry: the repository key carries no
/// second path, and the store location has been `%ProgramFiles%\WindowsApps`
/// since packages exist.
fn windows_apps_dir(full_name: &str) -> Option<PathBuf> {
    let program_files = std::env::var_os("ProgramFiles")?;
    Some(
        PathBuf::from(program_files)
            .join("WindowsApps")
            .join(full_name),
    )
}

/// `PackageRootFolder` from the repository mirror.
fn package_root(full_name: &str) -> Option<PathBuf> {
    let key = CURRENT_USER
        .open(format!("{REPOSITORY_PACKAGES}\\{full_name}"))
        .ok()?;
    let root = key.get_string("PackageRootFolder").ok()?;
    (!root.is_empty()).then(|| PathBuf::from(root))
}

/// The blocked list of one hive, normalised like [`braced`] spells CLSIDs.
///
/// `Key::values()` rather than a raw enumeration: the value-iterator concern
/// from `rules/registry.md` is about *key* names; values here are short
/// GUIDs, and `clsid.rs` reads the same list the same way.
fn blocked_values(hive: &Key) -> rustc_hash::FxHashSet<String> {
    let mut out = rustc_hash::FxHashSet::default();
    if let Ok(key) = hive.open(SHELL_EXTENSIONS_BLOCKED)
        && let Ok(values) = key.values()
    {
        for (name, _) in values {
            out.insert(name.to_uppercase());
        }
    }
    out
}

/// A path from the manifest, made absolute.
///
/// Manifest paths are relative to the package root; an absolute one (rare,
/// but sparse packages can point anywhere) is kept as it is.
fn anchor(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// `{CLSID}` in upper case — the blocked list's spelling.
///
/// Manifests write the GUID bare, the blocked list writes it braced, and
/// both mix cases freely.
fn braced(clsid: &str) -> String {
    let trimmed = clsid.trim().trim_matches(|c| c == '{' || c == '}');
    format!("{{{}}}", trimmed.to_uppercase())
}

/// What one manifest declares, before registry state is mixed in.
#[derive(Debug, Default, PartialEq)]
struct ParsedManifest {
    display_name: Option<String>,
    logo: Option<String>,
    verbs: Vec<ParsedVerb>,
}

#[derive(Debug, PartialEq)]
struct ParsedVerb {
    id: String,
    clsid: String,
    item_types: Vec<String>,
    dll: Option<PathBuf>,
}

/// Reads the two extensions that make a context menu entry.
///
/// Matching is on local element names only. The manifest schema spells the
/// same element `desktop4:`, `desktop5:` or `desktop10:` depending on which
/// Windows version the package targets, and a parser bound to one prefix
/// would silently miss the others' packages.
///
/// Returns `None` only for XML that does not parse; a manifest without menu
/// extensions parses fine and comes back empty.
fn parse_manifest(xml: &str) -> Option<ParsedManifest> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let mut parsed = ParsedManifest::default();

    // CLSID -> server path, collected from every Class element under a
    // ComServer. Surrogate and exe servers differ in how they host the
    // class, not in how they declare it.
    let mut servers: Vec<(String, PathBuf)> = Vec::new();
    // CLSID -> (verb id, item types); one verb can serve several item types.
    let mut verbs: Vec<ParsedVerb> = Vec::new();

    for node in doc.descendants() {
        match node.tag_name().name() {
            // Only the child of `Properties` names the package; the other
            // `DisplayName` and `Logo` elements belong to applications and
            // visual elements.
            "DisplayName"
                if parsed.display_name.is_none()
                    && node
                        .parent()
                        .is_some_and(|p| p.tag_name().name() == "Properties") =>
            {
                parsed.display_name = node.text().map(str::to_string);
            }
            "Logo"
                if node
                    .parent()
                    .is_some_and(|p| p.tag_name().name() == "Properties") =>
            {
                parsed.logo = node.text().map(str::to_string);
            }
            "Class" => {
                let inside_com_server =
                    node.ancestors().any(|a| a.tag_name().name() == "ComServer");
                if !inside_com_server {
                    continue;
                }
                if let (Some(id), Some(path)) = (node.attribute("Id"), node.attribute("Path")) {
                    servers.push((braced(id), PathBuf::from(path)));
                }
            }
            "ItemType" => {
                let Some(item_type) = node.attribute("Type") else {
                    continue;
                };
                for verb in node.children() {
                    if verb.tag_name().name() != "Verb" {
                        continue;
                    }
                    let (Some(id), Some(clsid)) = (verb.attribute("Id"), verb.attribute("Clsid"))
                    else {
                        continue;
                    };
                    let clsid = braced(clsid);
                    match verbs.iter_mut().find(|v| v.clsid == clsid && v.id == id) {
                        Some(existing) => existing.item_types.push(item_type.to_string()),
                        None => verbs.push(ParsedVerb {
                            id: id.to_string(),
                            clsid,
                            item_types: vec![item_type.to_string()],
                            dll: None,
                        }),
                    }
                }
            }
            _ => {}
        }
    }

    for verb in &mut verbs {
        verb.dll = servers
            .iter()
            .find(|(clsid, _)| *clsid == verb.clsid)
            .map(|(_, path)| path.clone());
    }

    parsed.verbs = verbs;
    Some(parsed)
}

/// The package's display name, best effort.
///
/// Manifest display names of store packages are `ms-resource:` references
/// into the package's own resource index. `SHLoadIndirectString` resolves
/// those from any process when given the fully qualified form; which of the
/// documented spellings a package uses varies, so several are tried. When
/// none resolves, the family part of the full name — `Microsoft.WindowsNotepad`
/// out of `Microsoft.WindowsNotepad_11.2312.18.0_x64__8wekyb3d8bbwe` — is a
/// readable fallback, unlike the raw reference.
fn display_name(full_name: &str, manifest_name: Option<&str>, mui: &mut MuiResolver) -> String {
    let fallback = || family_name(full_name).to_string();

    let Some(raw) = manifest_name else {
        return fallback();
    };
    if !raw.starts_with("ms-resource:") {
        return raw.to_string();
    }

    for candidate in ms_resource_candidates(full_name, raw) {
        let resolved = mui.resolve(&candidate);
        if resolved != candidate && !resolved.trim().is_empty() {
            return resolved;
        }
    }
    fallback()
}

/// The fully qualified spellings under which a package resource may resolve.
fn ms_resource_candidates(full_name: &str, raw: &str) -> Vec<String> {
    let name = family_name(full_name);
    let tail = raw.trim_start_matches("ms-resource:");

    if tail.starts_with("//") {
        // Already fully qualified inside the reference.
        return vec![format!("@{{{full_name}?{raw}}}")];
    }
    vec![
        format!("@{{{full_name}?ms-resource://{name}/{tail}}}"),
        format!("@{{{full_name}?ms-resource://{name}/resources/{tail}}}"),
    ]
}

/// `Microsoft.WindowsNotepad` out of the full name.
fn family_name(full_name: &str) -> &str {
    full_name.split('_').next().unwrap_or(full_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest shape the prototype proved on 2026-08-20: desktop4/5
    /// prefixes, a surrogate server, one verb for `*`.
    const PROTO_MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:com="http://schemas.microsoft.com/appx/manifest/com/windows10"
  xmlns:desktop4="http://schemas.microsoft.com/appx/manifest/desktop/windows10/4"
  xmlns:desktop5="http://schemas.microsoft.com/appx/manifest/desktop/windows10/5">
  <Identity Name="CtxmenuProto" ProcessorArchitecture="x64" Publisher="CN=x" Version="0.1.0.0" />
  <Properties>
    <DisplayName>ctxmenu Prototyp</DisplayName>
    <PublisherDisplayName>ctxmenu</PublisherDisplayName>
    <Logo>proto.png</Logo>
  </Properties>
  <Applications>
    <Application Id="CtxmenuProto" Executable="ctxmenu.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="visual name, not the package's" Description="d"
        Square150x150Logo="p.png" Square44x44Logo="p.png" BackgroundColor="transparent" />
      <Extensions>
        <com:Extension Category="windows.comServer">
          <com:ComServer>
            <com:SurrogateServer DisplayName="ctxmenu Prototyp">
              <com:Class Id="4103969a-91a4-45ab-8521-12f32d897bbc" Path="ctxmenu_proto.dll" ThreadingModel="STA" />
            </com:SurrogateServer>
          </com:ComServer>
        </com:Extension>
        <desktop4:Extension Category="windows.fileExplorerContextMenus">
          <desktop4:FileExplorerContextMenus>
            <desktop5:ItemType Type="*">
              <desktop5:Verb Id="CtxmenuProto" Clsid="4103969A-91A4-45AB-8521-12F32D897BBC" />
            </desktop5:ItemType>
            <desktop5:ItemType Type="Directory">
              <desktop5:Verb Id="CtxmenuProto" Clsid="4103969A-91A4-45AB-8521-12F32D897BBC" />
            </desktop5:ItemType>
          </desktop4:FileExplorerContextMenus>
        </desktop4:Extension>
      </Extensions>
    </Application>
  </Applications>
</Package>"#;

    #[test]
    fn the_prototype_manifest_yields_one_verb_for_two_item_types() {
        let parsed = parse_manifest(PROTO_MANIFEST).expect("well-formed XML");

        assert_eq!(parsed.display_name.as_deref(), Some("ctxmenu Prototyp"));
        assert_eq!(parsed.logo.as_deref(), Some("proto.png"));
        assert_eq!(parsed.verbs.len(), 1, "same verb, merged across item types");

        let verb = &parsed.verbs[0];
        assert_eq!(verb.id, "CtxmenuProto");
        assert_eq!(verb.clsid, "{4103969A-91A4-45AB-8521-12F32D897BBC}");
        assert_eq!(verb.item_types, vec!["*", "Directory"]);
        assert_eq!(verb.dll.as_deref(), Some(Path::new("ctxmenu_proto.dll")));
    }

    #[test]
    fn the_package_display_name_wins_over_the_visual_one() {
        // Both are `DisplayName` elements; only the child of `Properties`
        // names the package.
        let parsed = parse_manifest(PROTO_MANIFEST).unwrap();
        assert_eq!(parsed.display_name.as_deref(), Some("ctxmenu Prototyp"));
    }

    #[test]
    fn a_newer_schema_prefix_parses_the_same() {
        // The item types under desktop10 while the extension stays desktop4 —
        // a mix that really occurs. The local names carry the meaning, the
        // prefix only carries the schema version. (Replacing both prefixes
        // would declare xmlns:desktop10 twice, which is not XML any more.)
        let manifest = PROTO_MANIFEST.replace("desktop5", "desktop10");
        let parsed = parse_manifest(&manifest).expect("well-formed XML");
        assert_eq!(parsed.verbs.len(), 1);
        assert_eq!(parsed.verbs[0].item_types, vec!["*", "Directory"]);
    }

    #[test]
    fn a_manifest_without_menu_extensions_comes_back_empty_not_none() {
        let manifest = r#"<?xml version="1.0"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Properties><DisplayName>Plain</DisplayName></Properties>
</Package>"#;
        let parsed = parse_manifest(manifest).expect("well-formed XML");
        assert_eq!(parsed.display_name.as_deref(), Some("Plain"));
        assert!(parsed.verbs.is_empty());
    }

    #[test]
    fn broken_xml_is_none_rather_than_a_panic() {
        assert_eq!(parse_manifest("<Package><unclosed"), None);
    }

    #[test]
    fn clsids_are_normalised_to_the_blocked_lists_spelling() {
        assert_eq!(
            braced("ca6cc9f1-867a-481e-951e-a28c5e4f01ea"),
            "{CA6CC9F1-867A-481E-951E-A28C5E4F01EA}"
        );
        assert_eq!(braced("{ABC}"), "{ABC}");
        assert_eq!(braced(" {abc} "), "{ABC}");
    }

    #[test]
    fn resource_candidates_carry_the_full_package_name() {
        let candidates = ms_resource_candidates(
            "Microsoft.WindowsNotepad_11.2312.18.0_x64__8wekyb3d8bbwe",
            "ms-resource:Resources/AppStoreName",
        );
        assert_eq!(
            candidates[0],
            "@{Microsoft.WindowsNotepad_11.2312.18.0_x64__8wekyb3d8bbwe?ms-resource://Microsoft.WindowsNotepad/Resources/AppStoreName}"
        );
        assert!(candidates[1].contains("/resources/Resources/AppStoreName"));
    }

    #[test]
    fn an_already_qualified_reference_is_passed_through_once() {
        let candidates =
            ms_resource_candidates("Pkg_1.0_x64__hash", "ms-resource://Pkg/Resources/Name");
        assert_eq!(
            candidates,
            vec!["@{Pkg_1.0_x64__hash?ms-resource://Pkg/Resources/Name}"]
        );
    }

    #[test]
    fn the_family_name_is_the_part_before_the_first_underscore() {
        assert_eq!(
            family_name("Microsoft.WindowsTerminal_1.18.10301.0_x64__8wekyb3d8bbwe"),
            "Microsoft.WindowsTerminal"
        );
        assert_eq!(family_name("NoUnderscore"), "NoUnderscore");
    }

    /// A menu with one package and one verb over two item types, the shape
    /// the prototype proved.
    fn fixture() -> PackagedMenu {
        PackagedMenu {
            packages: vec![PackagedPackage {
                full_name: "CtxmenuProto_0.1.0.0_x64__abc".into(),
                display_name: "ctxmenu Prototyp".into(),
                root: PathBuf::from(r"C:\somewhere"),
                logo: None,
                scope: Scope::User,
                verbs: vec![PackagedVerb {
                    id: "CtxmenuProto".into(),
                    clsid: "{4103969A-91A4-45AB-8521-12F32D897BBC}".into(),
                    item_types: vec!["*".into(), ".png".into(), "Wobbly".into()],
                    dll: Some(PathBuf::from(r"C:\somewhere\ctxmenu_proto.dll")),
                    blocked_user: true,
                    blocked_machine: false,
                }],
            }],
            skipped: 0,
        }
    }

    #[test]
    fn one_entry_per_item_type_and_the_unknown_one_is_dropped() {
        let entries = entries(&fixture());
        assert_eq!(entries.len(), 2, "'*' and '.png'; 'Wobbly' has no category");
        assert_eq!(entries[0].category, crate::model::Category::AllFiles);
        assert_eq!(
            entries[1].category,
            crate::model::Category::ExtAssoc(".png".into())
        );
        assert_ne!(entries[0].id, entries[1].id, "two rows, two ids");
    }

    #[test]
    fn the_user_block_is_what_hidden_means_here() {
        let entries = entries(&fixture());
        assert!(entries[0].hidden);
        assert!(!entries[0].read_only, "the HKCU blocked list is writable");
        assert_eq!(entries[0].applies_to.as_deref(), Some("*"));
    }

    /// The carrier path must be a valid target: the plan path parses it to
    /// learn scope and display path, even though it never writes it.
    #[test]
    fn the_carrier_path_parses_as_a_target() {
        let entries = entries(&fixture());
        let target = super::super::paths::RegTarget::parse(&entries[0].registry_path)
            .expect("PackagedCom lives below Classes");
        assert_eq!(target.scope(), Scope::User);
        assert_eq!(
            entries[0].registry_path,
            r"HKCU\SOFTWARE\Classes\PackagedCom\Package\CtxmenuProto_0.1.0.0_x64__abc"
        );
    }

    /// The scan must come back on every machine this runs on — Windows 10
    /// has `PackagedCom` too, it only lacks the menu that reads it.
    #[test]
    fn scanning_never_panics_whatever_is_installed() {
        let menu = scan();
        for package in &menu.packages {
            assert!(!package.full_name.is_empty());
            assert!(!package.verbs.is_empty(), "verbless packages are dropped");
            for verb in &package.verbs {
                assert!(verb.clsid.starts_with('{') && verb.clsid.ends_with('}'));
            }
        }
    }
}
