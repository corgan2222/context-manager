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

/// Live objects handed out by this DLL, plus outstanding `LockServer` locks.
///
/// The number `DllCanUnloadNow` answers from. Until 1.5.1 that function
/// returned `S_FALSE` unconditionally, which says "never unload me": the
/// `dllhost` surrogate then kept the DLL mapped for its own lifetime, and a
/// mapped file out of the package folder is a file Windows cannot clear away
/// when the package is unregistered — the folder goes to
/// `WindowsApps\Deleted\` and pieces of the registration stay behind.
/// Whether that is what left the stale `PackagedCom` key on the 2026-08-21
/// VM was not proven; that the unconditional answer breaks the COM contract
/// was.
static ALIVE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Counts one live object for as long as it exists.
///
/// A field in every type this DLL hands to the shell, so the count follows
/// the objects without any call site having to remember it.
///
/// **Construct it with [`Alive::new`], never as the bare literal `Alive`.**
/// It is a unit struct, so `Alive` compiles anywhere `Alive::new()` does —
/// and counts nothing, because the counting happens in the constructor. The
/// same reason rules out `Default::default()` at a call site: clippy's
/// `default_constructed_unit_structs` would tell the next reader to simplify
/// it to `Alive`, which is the one edit that silently breaks this file.
/// `Default` still exists so `RootCommand` can go on deriving it.
#[derive(Debug)]
struct Alive;

impl Alive {
    fn new() -> Self {
        ALIVE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}

impl Default for Alive {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Alive {
    fn drop(&mut self) {
        ALIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

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

/// An extension in the form a selection reports it: lowercase, dot included.
///
/// Records written before 1.5.1 can hold a dotless extension. The window
/// normalised on its way into the registry but not on its way into
/// `entries.json`, so somebody who typed `png` has `{"ExtAssoc":"png"}` on
/// disk while [`selection`] builds `".png"` from the file itself. Comparing
/// those two hides the entry, which is exactly what it did.
///
/// The window fixes the writing side, but a file already on disk is not
/// rewritten until its entry is touched again, and this DLL has no business
/// rewriting the user's file to make a menu appear. So it reads both forms.
fn dotted(raw: &str) -> String {
    let bare = raw.trim().trim_start_matches('*').trim_start_matches('.');
    format!(".{}", bare.to_lowercase())
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
            return Applies::Ext(dotted(ext));
        }
        if let Some(prog) = object.get("ProgId")
            && let Some(ext) = prog.get("from_ext").and_then(|v| v.as_str())
        {
            return Applies::Ext(dotted(ext));
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
    _alive: Alive,
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
    _alive: Alive,
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
                    _alive: Alive::new(),
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
    _alive: Alive,
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
                    _alive: Alive::new(),
                }
                .into(),
                false => SubmenuCommand {
                    display_name: entry.display_name,
                    icon: entry.icon,
                    category: entry.category,
                    children: entry.children,
                    _alive: Alive::new(),
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
    _alive: Alive,
}

impl CommandEnum {
    fn new(commands: Vec<IExplorerCommand>) -> Self {
        Self {
            commands,
            position: std::cell::Cell::new(0),
            _alive: Alive::new(),
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
    fn LockServer(&self, lock: BOOL) -> Result<()> {
        // A lock is a caller saying "keep the server alive even with no
        // objects out". It counts the same as an object, which is the whole
        // reason `ALIVE` is an isize and not a usize: an unbalanced unlock
        // must go negative and be visible, not wrap around into a number
        // that claims two billion live objects.
        match lock.as_bool() {
            true => ALIVE.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            false => ALIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst),
        };
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

/// May COM unload this DLL?
///
/// `S_OK` once nothing of ours is alive any more, `S_FALSE` while something
/// is. The contract, in other words — and what this function did not do
/// until 1.5.1, when it answered `S_FALSE` no matter what and so told every
/// host it could never be unloaded. See [`ALIVE`] for what that costs.
///
/// A negative count would mean more releases than acquisitions, which is a
/// bug in this file rather than a reason to keep the DLL loaded; `<= 0`
/// therefore reads as "nothing alive" instead of pretending otherwise.
#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    match ALIVE.load(std::sync::atomic::Ordering::SeqCst) <= 0 {
        true => S_OK,
        false => S_FALSE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises the two tests that read `ALIVE` as an absolute number.
    ///
    /// `cargo test` runs threads in parallel and the counter is global, so
    /// without this each of them would see the other's objects. Poisoning is
    /// ignored deliberately: a panic in one of them must not turn the other
    /// into a second, confusing failure.
    static COUNTING: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn value(raw: &str) -> serde_json::Value {
        serde_json::from_str(raw).expect("test JSON parses")
    }

    /// Regression: a record written before 1.5.1 can carry `png` where the
    /// selection reports `.png`. Comparing the two literally hid the entry
    /// from the new menu while the classic menu showed it, because only the
    /// registry path had been normalised.
    #[test]
    fn an_extension_recorded_without_its_dot_still_matches() {
        for raw in [
            r#"{"ExtAssoc": "png"}"#,
            r#"{"ExtAssoc": "PNG"}"#,
            r#"{"ExtAssoc": "*.png"}"#,
            r#"{"ExtDirect": "png"}"#,
            r#"{"ProgId": {"prog_id": "pngfile", "from_ext": "png"}}"#,
        ] {
            assert!(
                matches(&entry_with(raw), &files_only()),
                "an entry recorded as {raw} must reach a .png selection"
            );
        }
        assert!(
            !matches(&entry_with(r#"{"ExtAssoc": "jpg"}"#), &files_only()),
            "tolerance about the dot is not tolerance about the extension"
        );
    }

    /// Regression for 1.5.0, where `DllCanUnloadNow` answered `S_FALSE`
    /// unconditionally: the surrogate then held the DLL — and with it a file
    /// out of the package folder — for its whole lifetime.
    #[test]
    fn a_live_object_holds_the_dll_and_dropping_it_lets_go() {
        let _serial = COUNTING.lock().unwrap_or_else(|held| held.into_inner());
        assert_eq!(DllCanUnloadNow(), S_OK, "nothing of ours is alive yet");

        let command: IExplorerCommand = RootCommand::default().into();
        assert_eq!(
            DllCanUnloadNow(),
            S_FALSE,
            "an object is out; unloading now would pull the code from under it"
        );

        drop(command);
        assert_eq!(DllCanUnloadNow(), S_OK, "the last object is gone");
    }

    /// Every type handed to the shell carries the count, not just the root.
    ///
    /// A leaf and a submenu outlive the enumerator that produced them — the
    /// shell holds them for as long as the menu is up. Each is built from a
    /// struct literal, where `Alive::new()` is one careless simplification
    /// away from the bare `Alive` that counts nothing. This is what notices.
    #[test]
    fn a_leaf_and_a_submenu_hold_the_dll_as_the_root_does() {
        let _serial = COUNTING.lock().unwrap_or_else(|held| held.into_inner());
        assert_eq!(DllCanUnloadNow(), S_OK, "nothing of ours is alive yet");

        let leaf: IExplorerCommand = LeafCommand {
            display_name: "leaf".into(),
            command: String::new(),
            icon: None,
            category: value(r#""AllFiles""#),
            _alive: Alive::new(),
        }
        .into();
        assert_eq!(DllCanUnloadNow(), S_FALSE, "the leaf is out");

        let submenu: IExplorerCommand = SubmenuCommand {
            display_name: "submenu".into(),
            icon: None,
            category: value(r#""AllFiles""#),
            children: Vec::new(),
            _alive: Alive::new(),
        }
        .into();

        drop(leaf);
        assert_eq!(
            DllCanUnloadNow(),
            S_FALSE,
            "the submenu is still out; one release must not free the DLL"
        );

        drop(submenu);
        assert_eq!(DllCanUnloadNow(), S_OK, "both are gone");
    }

    /// A `LockServer(TRUE)` with no objects out has to hold the DLL just as
    /// an object does — that is the entire purpose of the call.
    #[test]
    fn a_server_lock_holds_the_dll_on_its_own() {
        let _serial = COUNTING.lock().unwrap_or_else(|held| held.into_inner());
        let factory: IClassFactory = Factory.into();
        assert_eq!(
            DllCanUnloadNow(),
            S_OK,
            "a class object is the host's to hold, not ours to count"
        );

        unsafe { factory.LockServer(true) }.expect("locking succeeds");
        assert_eq!(DllCanUnloadNow(), S_FALSE, "the lock holds the DLL");

        unsafe { factory.LockServer(false) }.expect("unlocking succeeds");
        assert_eq!(DllCanUnloadNow(), S_OK, "the lock is released");
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
