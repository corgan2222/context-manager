//! What the new Windows 11 menu shows for ctxmenu's own entries.
//!
//! The window writes every entry it creates into `entries.json`
//! (`ctxmenu/src/registry/create.rs` documents the format and its
//! stability promise). This DLL is the other reader: the shell loads it
//! through the sparse package's `windows.comServer` registration, asks the
//! root command for its state and children, and draws one `ctxmenu` flyout
//! with the recorded entries inside. Windows itself forces the flyout as
//! soon as an app brings more than one verb — the shape is the platform's,
//! not a choice.
//!
//! The DLL reads the file on every menu build instead of caching: the menu
//! opens rarely, the file is small, and a cache would show yesterday's
//! entries after the window wrote new ones.
//!
//! Deliberately not linked against the `ctxmenu` crate: the shell loads
//! this into a `dllhost.exe` surrogate, and a menu handler that pulls a GUI
//! stack in with it is the kind of neighbour nobody wants. The reading
//! structs are a tolerant duplicate of `NewEntry`, unknown fields ignored.

#![allow(non_snake_case)]

use std::path::PathBuf;

use serde::Deserialize;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::SystemServices::SFGAO_FOLDER;
use windows::Win32::UI::Shell::*;
use windows::core::*;

/// Must match the `Clsid` in the sparse package's `AppxManifest.xml`.
const CLSID_HANDLER: GUID = GUID::from_u128(0xC898E0C0_879E_4A3E_AF7E_631D99C7DE44);

/// What this DLL needs to know about one recorded entry.
///
/// A tolerant twin of `NewEntry`: only the fields the menu needs, unknown
/// ones skipped, so the window's format can grow without breaking a DLL
/// that is already registered.
#[derive(Debug, Clone, Deserialize)]
struct Entry {
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    icon: Option<String>,
    /// Kept as raw JSON: the category enum belongs to the window, and an
    /// unknown variant must degrade to "show it", not to a parse error.
    #[serde(default)]
    category: serde_json::Value,
    #[serde(default)]
    children: Vec<Child>,
}

#[derive(Debug, Clone, Deserialize)]
struct Child {
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    icon: Option<String>,
}

fn entries_path() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("ctxmenu").join("entries.json"))
}

/// Everything the window has recorded, or nothing.
///
/// Every failure mode is an empty list: no file means nothing created, and
/// a damaged file is the window's problem to report — a context menu is no
/// place for an error dialog.
///
/// Element by element, not all or nothing: the window's reader keeps an
/// element it cannot parse in the file on purpose, and one such element
/// must cost the menu that element, never the whole list.
fn read_entries() -> Vec<Entry> {
    let Some(path) = entries_path() else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_entries(&raw)
}

fn parse_entries(raw: &str) -> Vec<Entry> {
    let Ok(elements) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {
        return Vec::new();
    };
    elements
        .into_iter()
        .filter_map(|element| serde_json::from_value(element).ok())
        .collect()
}

/// Where an entry wants to appear, reduced to what a selection can answer.
enum Applies {
    Files,
    Everything,
    Folders,
    Ext(String),
    /// A perceived type such as `image`; resolved per extension against the
    /// registry when the menu opens.
    Perceived(String),
}

/// The window's `Category` as this DLL needs it.
///
/// Unknown variants fall back to `Everything`: a new category in a newer
/// window must not make an already-registered DLL hide the entry.
fn applies(category: &serde_json::Value) -> Applies {
    if let Some(name) = category.as_str() {
        return match name {
            "AllFiles" => Applies::Files,
            // Files no program has claimed: never a folder, so files are the
            // honest approximation the selection can answer.
            "Unknown" => Applies::Files,
            "Directory" | "Folder" | "Drive" => Applies::Folders,
            // A background click hands the folder itself to the handler, so
            // folders are the closest a selection-based answer gets.
            "DirectoryBackground" | "DesktopBackground" => Applies::Folders,
            // The folder-content menus: which template a folder carries is
            // the shell's knowledge, folders are the honest approximation.
            "DirectoryAudio" | "DirectoryImage" | "DirectoryVideo" => Applies::Folders,
            _ => Applies::Everything,
        };
    }
    if let Some(object) = category.as_object() {
        if let Some(ext) = object.get("ExtAssoc").or(object.get("ExtDirect"))
            && let Some(ext) = ext.as_str()
        {
            return Applies::Ext(ext.to_lowercase());
        }
        if let Some(prog) = object.get("ProgId")
            && let Some(ext) = prog.get("from_ext").and_then(|v| v.as_str())
        {
            return Applies::Ext(ext.to_lowercase());
        }
        if let Some(perceived) = object.get("PerceivedType").and_then(|v| v.as_str()) {
            return Applies::Perceived(perceived.to_lowercase());
        }
    }
    Applies::Everything
}

/// The selection, reduced to the two questions `applies` asks.
struct Selection {
    any_folder: bool,
    any_file: bool,
    /// Lowercased extension of the first file, dot included.
    first_ext: Option<String>,
    /// Filesystem paths of every item, for `Invoke`.
    paths: Vec<String>,
}

fn selection(items: Ref<'_, IShellItemArray>) -> Selection {
    let mut out = Selection {
        any_folder: false,
        any_file: false,
        first_ext: None,
        paths: Vec::new(),
    };
    let Some(items) = items.as_ref() else {
        return out;
    };
    let count = unsafe { items.GetCount() }.unwrap_or(0);
    for index in 0..count {
        let Ok(item) = (unsafe { items.GetItemAt(index) }) else {
            continue;
        };
        let folder = unsafe { item.GetAttributes(SFGAO_FOLDER) }
            .map(|got| got.contains(SFGAO_FOLDER))
            .unwrap_or(false);
        if folder {
            out.any_folder = true;
        } else {
            out.any_file = true;
        }
        if let Ok(path) = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) } {
            let path = unsafe { path.to_string() }.unwrap_or_default();
            if !folder && out.first_ext.is_none() {
                out.first_ext = std::path::Path::new(&path)
                    .extension()
                    .map(|ext| format!(".{}", ext.to_string_lossy().to_lowercase()));
            }
            if !path.is_empty() {
                out.paths.push(path);
            }
        }
    }
    out
}

/// The `PerceivedType` of an extension, the way the shell reads it.
fn perceived_type_of(ext: &str) -> Option<String> {
    windows_registry::CLASSES_ROOT
        .open(ext)
        .ok()?
        .get_string("PerceivedType")
        .ok()
        .map(|value| value.to_lowercase())
}

fn matches(entry: &Entry, selection: &Selection) -> bool {
    match applies(&entry.category) {
        Applies::Everything => true,
        Applies::Files => selection.any_file,
        Applies::Folders => selection.any_folder,
        Applies::Ext(ext) => selection.first_ext.as_deref() == Some(ext.as_str()),
        Applies::Perceived(perceived) => {
            selection
                .first_ext
                .as_deref()
                .and_then(perceived_type_of)
                .as_deref()
                == Some(perceived.as_str())
        }
    }
}

/// Starts the entry's command line for the clicked selection.
///
/// `%1` and `%V` are what the window writes into recorded commands — the
/// path placeholder of a shell verb, `%V` in the two background categories.
/// One start per selected item, which is what the classic menu does for a
/// multi-selection too.
///
/// Through `ShellExecuteExW`, not `CreateProcessW`: a recorded command may
/// name a `.lnk` — the entry editor's file picker happily takes one — and
/// only the shell resolves shortcuts. The line is split into program and
/// arguments first, because ShellExecute wants them apart.
fn run(command: &str, selection: &Selection) -> Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let paths: &[String] = if selection.paths.is_empty() {
        &[String::new()]
    } else {
        &selection.paths
    };
    for path in paths {
        let line = command.replace("%1", path).replace("%V", path);
        let expanded = expand_env(&line);
        let (file, parameters) = split_command(&expanded);

        let file: Vec<u16> = file.encode_utf16().chain(std::iter::once(0)).collect();
        let parameters: Vec<u16> = parameters
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            // NOASYNC because this thread belongs to a dllhost that may be
            // torn down right after Invoke returns.
            fMask: SEE_MASK_NOASYNC,
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(parameters.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        unsafe { ShellExecuteExW(&mut info) }?;
    }
    Ok(())
}

/// Splits one command line into the program and everything behind it.
///
/// A quoted program keeps its spaces; the quotes themselves are not part of
/// the name ShellExecute wants.
fn split_command(line: &str) -> (String, String) {
    let line = line.trim_start();
    if let Some(rest) = line.strip_prefix('"') {
        match rest.split_once('"') {
            Some((file, parameters)) => (file.to_string(), parameters.trim().to_string()),
            None => (rest.to_string(), String::new()),
        }
    } else {
        match line.split_once(' ') {
            Some((file, parameters)) => (file.to_string(), parameters.trim().to_string()),
            None => (line.to_string(), String::new()),
        }
    }
}

fn expand_env(raw: &str) -> String {
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
    let source = HSTRING::from(raw);
    unsafe {
        let needed = ExpandEnvironmentStringsW(&source, None);
        if needed == 0 {
            return raw.to_string();
        }
        let mut buffer = vec![0u16; needed as usize];
        let written = ExpandEnvironmentStringsW(&source, Some(&mut buffer));
        if written == 0 || written as usize > buffer.len() {
            return raw.to_string();
        }
        String::from_utf16_lossy(&buffer[..written as usize - 1])
    }
}

fn title(text: &str) -> Result<PWSTR> {
    unsafe { SHStrDupW(&HSTRING::from(text)) }
}

/// One leaf: a recorded entry, or one child of a recorded submenu.
#[implement(IExplorerCommand)]
struct LeafCommand {
    display_name: String,
    command: String,
    icon: Option<String>,
    /// The parent's category; a child applies wherever its submenu does.
    category: serde_json::Value,
}

impl IExplorerCommand_Impl for LeafCommand_Impl {
    fn GetTitle(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        title(&self.display_name)
    }
    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        match &self.icon {
            Some(icon) => title(icon),
            None => Err(E_NOTIMPL.into()),
        }
    }
    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }
    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(GUID::zeroed())
    }
    fn GetState(&self, items: Ref<IShellItemArray>, _slow_ok: BOOL) -> Result<u32> {
        let entry = Entry {
            display_name: String::new(),
            command: String::new(),
            icon: None,
            category: self.category.clone(),
            children: Vec::new(),
        };
        Ok(match matches(&entry, &selection(items)) {
            true => ECS_ENABLED.0 as u32,
            false => ECS_HIDDEN.0 as u32,
        })
    }
    fn Invoke(&self, items: Ref<IShellItemArray>, _bctx: Ref<IBindCtx>) -> Result<()> {
        if self.command.is_empty() {
            return Ok(());
        }
        run(&self.command, &selection(items))
    }
    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }
    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}

/// A recorded submenu: its children become the second flyout level.
///
/// Whether the new menu draws a second level is the platform's call — the
/// declaration is correct either way, and the first level always works.
#[implement(IExplorerCommand)]
struct SubmenuCommand {
    display_name: String,
    icon: Option<String>,
    category: serde_json::Value,
    children: Vec<Child>,
}

impl IExplorerCommand_Impl for SubmenuCommand_Impl {
    fn GetTitle(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        title(&self.display_name)
    }
    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        match &self.icon {
            Some(icon) => title(icon),
            None => Err(E_NOTIMPL.into()),
        }
    }
    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }
    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(GUID::zeroed())
    }
    fn GetState(&self, items: Ref<IShellItemArray>, _slow_ok: BOOL) -> Result<u32> {
        let entry = Entry {
            display_name: String::new(),
            command: String::new(),
            icon: None,
            category: self.category.clone(),
            children: Vec::new(),
        };
        Ok(match matches(&entry, &selection(items)) {
            true => ECS_ENABLED.0 as u32,
            false => ECS_HIDDEN.0 as u32,
        })
    }
    fn Invoke(&self, _items: Ref<IShellItemArray>, _bctx: Ref<IBindCtx>) -> Result<()> {
        Ok(())
    }
    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_HASSUBCOMMANDS.0 as u32)
    }
    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        let commands = self
            .children
            .iter()
            .map(|child| {
                LeafCommand {
                    display_name: child.display_name.clone(),
                    command: child.command.clone(),
                    icon: child.icon.clone(),
                    category: self.category.clone(),
                }
                .into()
            })
            .collect();
        Ok(CommandEnum::new(commands).into())
    }
}

/// The root: the one verb the manifest declares. Everything below it is
/// built from `entries.json` when the menu opens.
#[implement(IExplorerCommand)]
#[derive(Default)]
struct RootCommand {
    /// What the last look at the selection found: the entries that apply,
    /// and the selection itself for a later `Invoke`. Interior mutability
    /// is sound here — the manifest declares STA, one thread owns this
    /// object for one menu build.
    matching: std::cell::RefCell<Vec<Entry>>,
    selection: std::cell::RefCell<Option<Selection>>,
}

impl RootCommand_Impl {
    /// Reads the file and filters against the selection, once per menu.
    ///
    /// Every selection-carrying call funnels through here, because the one
    /// call that decides the shape — `GetFlags` — carries no selection and
    /// can only reuse what an earlier call saw. If the shell ever asks for
    /// the flags first, the caches are empty and the answer degrades to the
    /// flyout, which is the shape that is never wrong.
    fn ensure(&self, items: Ref<IShellItemArray>) {
        if self.selection.borrow().is_some() {
            return;
        }
        let selection = selection(items);
        *self.matching.borrow_mut() = read_entries()
            .into_iter()
            .filter(|entry| matches(entry, &selection))
            .collect();
        *self.selection.borrow_mut() = Some(selection);
    }

    /// The single applying entry, when there is exactly one without
    /// children — the case the menu shows directly instead of as a flyout
    /// of one (asked for on 2026-08-20).
    fn only(&self) -> Option<Entry> {
        let matching = self.matching.borrow();
        match &matching[..] {
            [entry] if entry.children.is_empty() => Some(entry.clone()),
            _ => None,
        }
    }
}

impl IExplorerCommand_Impl for RootCommand_Impl {
    fn GetTitle(&self, items: Ref<IShellItemArray>) -> Result<PWSTR> {
        self.ensure(items);
        match self.only() {
            Some(entry) => title(&entry.display_name),
            None => title("ctxmenu"),
        }
    }
    fn GetIcon(&self, items: Ref<IShellItemArray>) -> Result<PWSTR> {
        self.ensure(items);
        match self.only().and_then(|entry| entry.icon) {
            Some(icon) => title(&icon),
            None => Err(E_NOTIMPL.into()),
        }
    }
    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }
    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(CLSID_HANDLER)
    }
    fn GetState(&self, items: Ref<IShellItemArray>, _slow_ok: BOOL) -> Result<u32> {
        // Hidden rather than an empty flyout: with nothing recorded, or
        // nothing that applies here, there is no ctxmenu entry to show.
        self.ensure(items);
        Ok(match self.matching.borrow().is_empty() {
            false => ECS_ENABLED.0 as u32,
            true => ECS_HIDDEN.0 as u32,
        })
    }
    fn Invoke(&self, items: Ref<IShellItemArray>, _bctx: Ref<IBindCtx>) -> Result<()> {
        self.ensure(items);
        let Some(entry) = self.only() else {
            return Ok(());
        };
        if entry.command.is_empty() {
            return Ok(());
        }
        match self.selection.borrow().as_ref() {
            Some(selection) => run(&entry.command, selection),
            None => Ok(()),
        }
    }
    fn GetFlags(&self) -> Result<u32> {
        // No selection reaches this call; the shape comes from what
        // `ensure` cached. An empty cache means the flyout — never wrong,
        // merely one click longer.
        Ok(match self.only() {
            Some(_) => ECF_DEFAULT.0 as u32,
            None => ECF_HASSUBCOMMANDS.0 as u32,
        })
    }
    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        // The filtered list when a selection was seen, everything recorded
        // otherwise — the sub-commands hide themselves per selection anyway.
        let entries = match self.selection.borrow().is_some() {
            true => self.matching.borrow().clone(),
            false => read_entries(),
        };
        let commands = entries
            .into_iter()
            .map(|entry| match entry.children.is_empty() {
                true => LeafCommand {
                    display_name: entry.display_name,
                    command: entry.command,
                    icon: entry.icon,
                    category: entry.category,
                }
                .into(),
                false => SubmenuCommand {
                    display_name: entry.display_name,
                    icon: entry.icon,
                    category: entry.category,
                    children: entry.children,
                }
                .into(),
            })
            .collect();
        Ok(CommandEnum::new(commands).into())
    }
}

/// The enumerator both flyout levels hand out.
#[implement(IEnumExplorerCommand)]
struct CommandEnum {
    commands: Vec<IExplorerCommand>,
    position: std::cell::Cell<usize>,
}

impl CommandEnum {
    fn new(commands: Vec<IExplorerCommand>) -> Self {
        Self {
            commands,
            position: std::cell::Cell::new(0),
        }
    }
}

impl IEnumExplorerCommand_Impl for CommandEnum_Impl {
    fn Next(
        &self,
        count: u32,
        commands: *mut Option<IExplorerCommand>,
        fetched: *mut u32,
    ) -> HRESULT {
        let mut written = 0usize;
        let out = unsafe { std::slice::from_raw_parts_mut(commands, count as usize) };
        while written < count as usize {
            let index = self.position.get();
            let Some(command) = self.commands.get(index) else {
                break;
            };
            out[written] = Some(command.clone());
            self.position.set(index + 1);
            written += 1;
        }
        if !fetched.is_null() {
            unsafe { *fetched = written as u32 };
        }
        match written == count as usize {
            true => S_OK,
            false => S_FALSE,
        }
    }
    fn Skip(&self, count: u32) -> Result<()> {
        self.position.set(self.position.get() + count as usize);
        Ok(())
    }
    fn Reset(&self) -> Result<()> {
        self.position.set(0);
        Ok(())
    }
    fn Clone(&self) -> Result<IEnumExplorerCommand> {
        Ok(CommandEnum::new(self.commands.clone()).into())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        object: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let command: IExplorerCommand = RootCommand::default().into();
        unsafe { command.query(iid, object).ok() }
    }
    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    object: *mut *mut core::ffi::c_void,
) -> HRESULT {
    if unsafe { *clsid } != CLSID_HANDLER {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = Factory.into();
    unsafe { factory.query(iid, object) }
}

#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(raw: &str) -> serde_json::Value {
        serde_json::from_str(raw).expect("test JSON parses")
    }

    fn files_only() -> Selection {
        Selection {
            any_folder: false,
            any_file: true,
            first_ext: Some(".png".into()),
            paths: vec![r"C:\x\a.png".into()],
        }
    }

    fn folders_only() -> Selection {
        Selection {
            any_folder: true,
            any_file: false,
            first_ext: None,
            paths: vec![r"C:\x".into()],
        }
    }

    fn entry(category: serde_json::Value) -> Entry {
        Entry {
            display_name: String::new(),
            command: String::new(),
            icon: None,
            category,
            children: Vec::new(),
        }
    }

    #[test]
    fn a_directory_entry_hides_on_a_file_and_shows_on_a_folder() {
        let entry = entry(value("\"Directory\""));
        assert!(!matches(&entry, &files_only()));
        assert!(matches(&entry, &folders_only()));
    }

    #[test]
    fn an_extension_entry_matches_its_extension_and_nothing_else() {
        let entry = entry(value(r#"{"ExtAssoc": ".PNG"}"#));
        assert!(matches(&entry, &files_only()), "case must not matter");
        assert!(!matches(&entry, &folders_only()));

        let other = entry_with(r#"{"ExtAssoc": ".txt"}"#);
        assert!(!matches(&other, &files_only()));
    }

    fn entry_with(raw: &str) -> Entry {
        entry(value(raw))
    }

    #[test]
    fn an_unknown_category_shows_rather_than_hides() {
        // A newer window may invent categories this DLL has never heard of;
        // the registered copy must keep showing the entry, not lose it.
        let entry = entry(value("\"SomethingFromTheFuture\""));
        assert!(matches(&entry, &files_only()));
        assert!(matches(&entry, &folders_only()));
    }

    #[test]
    fn the_recorded_format_reads_with_fields_this_dll_never_heard_of() {
        // The window's NewEntry carries more fields (key_name, position,
        // extended); this reader must take them in stride, in both
        // directions of format growth.
        let raw = r#"[{
            "category": {"ExtAssoc": ".png"},
            "key_name": "probe",
            "display_name": "Probe",
            "command": "notepad.exe \"%1\"",
            "icon": null,
            "position": "Top",
            "extended": false,
            "children": [],
            "invented_later": {"nested": true}
        }]"#;
        let entries: Vec<Entry> = serde_json::from_str(raw).expect("tolerant read");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].display_name, "Probe");
        assert_eq!(entries[0].command, "notepad.exe \"%1\"");
    }

    /// The case that found this: a favourite pointing at a `.lnk`, quoted
    /// because the path has spaces. ShellExecute wants program and
    /// arguments apart, and the quotes are not part of the program's name.
    #[test]
    fn a_command_line_splits_into_program_and_arguments() {
        assert_eq!(
            split_command(r#""C:\Users\P\Desktop\Visual Studio Code.lnk" "C:\a.txt""#),
            (
                r"C:\Users\P\Desktop\Visual Studio Code.lnk".to_string(),
                r#""C:\a.txt""#.to_string()
            ),
        );
        assert_eq!(
            split_command(r"notepad.exe C:\a.txt"),
            ("notepad.exe".to_string(), r"C:\a.txt".to_string())
        );
        assert_eq!(
            split_command("explorer.exe"),
            ("explorer.exe".to_string(), String::new()),
            "no arguments is not an error"
        );
    }

    #[test]
    fn the_placeholder_is_replaced_before_anything_runs() {
        // Only the string work, not the process start: what run() hands to
        // CreateProcessW is the part worth pinning.
        let line = "notepad.exe \"%1\"".replace("%1", r"C:\p\a.txt");
        assert_eq!(line, "notepad.exe \"C:\\p\\a.txt\"");
    }

    /// One element that is no entry at all — a hand-edited note, a stray
    /// string — must cost the menu that element, never the whole list. The
    /// window's reader keeps such an element in the file on purpose, so an
    /// all-or-nothing read here would blank the flyout for good.
    #[test]
    fn a_poison_element_costs_itself_not_the_list() {
        let raw = r#"["notiz", {"display_name": "Echt", "command": "C:\\t.exe", "category": "Directory"}, 42]"#;
        let entries = parse_entries(raw);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].display_name, "Echt");

        assert!(parse_entries("kein json").is_empty());
        assert!(parse_entries("[]").is_empty());
    }
}
