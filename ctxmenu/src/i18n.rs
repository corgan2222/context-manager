//! Interface strings in German and English.
//!
//! A plain struct of `&'static str`, not a framework: switching language is
//! then a single assignment that takes effect on the next frame — no setter,
//! no bindings, no restart (ToDo 8).
//!
//! Not translated, deliberately: registry paths, verb names, command lines,
//! and display names resolved through `SHLoadIndirectString`. Those already
//! arrive in the system language, and translating them would be inventing
//! data.

/// Every piece of text the interface shows.
///
/// Sentences that contain a number keep their placeholder and are filled with
/// `format!` at the call site. Pre-composing them would bake German word order
/// into the English strings.
pub struct Strings {
    pub app_title: &'static str,

    // Tabs
    pub tab_categories: &'static str,
    pub tab_filetypes: &'static str,
    pub tab_programs: &'static str,
    pub tab_backups: &'static str,

    // Categories
    pub cat_all_files: &'static str,
    pub cat_all_filesystem_objects: &'static str,
    pub cat_directory: &'static str,
    pub cat_directory_background: &'static str,
    pub cat_folder: &'static str,
    pub cat_desktop_background: &'static str,
    pub cat_drive: &'static str,

    // Table columns
    pub col_name: &'static str,
    pub col_type: &'static str,
    pub col_location: &'static str,
    pub col_scope: &'static str,
    pub col_command: &'static str,
    pub col_flags: &'static str,

    // Buttons
    pub btn_rescan: &'static str,
    pub btn_delete: &'static str,
    pub btn_disable: &'static str,
    pub btn_shift_only: &'static str,
    pub btn_block: &'static str,
    pub btn_restore: &'static str,
    pub btn_cancel: &'static str,
    pub btn_execute: &'static str,
    pub btn_select_all: &'static str,
    pub btn_select_none: &'static str,

    // Badges and states
    pub badge_admin: &'static str,
    pub badge_blocked: &'static str,
    pub badge_shift: &'static str,
    pub badge_hidden: &'static str,
    pub badge_readonly: &'static str,
    pub badge_system: &'static str,

    // Entry kinds
    pub kind_verb: &'static str,
    pub kind_shellex: &'static str,

    // Settings
    pub settings: &'static str,
    pub language: &'static str,
    pub theme: &'static str,
    pub theme_system: &'static str,
    pub theme_light: &'static str,
    pub theme_dark: &'static str,

    // Search and filters
    pub search_hint: &'static str,
    pub filter_hide_empty: &'static str,

    // Status and messages
    pub status_scanning: &'static str,
    pub status_ready: &'static str,
    pub status_elevated: &'static str,
    pub status_not_elevated: &'static str,
    pub msg_needs_admin: &'static str,
    pub msg_no_selection: &'static str,
    pub msg_confirm_delete: &'static str,
    pub msg_backup_first: &'static str,
    pub msg_com_handler_note: &'static str,
    pub msg_restart_explorer: &'static str,

    // Placeholder texts; {} is filled with format!
    pub fmt_entries_found: &'static str,
    pub fmt_scanning_location: &'static str,
    pub fmt_selected_count: &'static str,
    pub fmt_affects_other_types: &'static str,
    pub fmt_backup_created: &'static str,

    // Detail panel
    pub detail_title: &'static str,
    pub detail_registry_path: &'static str,
    pub detail_display_name: &'static str,
    pub detail_raw_value: &'static str,
    pub detail_icon: &'static str,
    pub detail_command: &'static str,
    pub detail_clsid: &'static str,
    pub detail_server: &'static str,
    pub detail_position: &'static str,
    pub detail_applies_to: &'static str,
    pub detail_nothing_selected: &'static str,

    // Editor for one's own entries
    pub editor_new: &'static str,
    pub editor_title: &'static str,
    pub editor_category: &'static str,
    pub editor_display_name: &'static str,
    pub editor_key_name: &'static str,
    pub editor_command: &'static str,
    pub editor_icon: &'static str,
    pub editor_position: &'static str,
    pub editor_visibility: &'static str,
    pub editor_extended: &'static str,
    pub editor_created_before: &'static str,
    pub editor_create: &'static str,
    pub pos_default: &'static str,
    pub pos_top: &'static str,
    pub pos_bottom: &'static str,
}

pub static DE: Strings = Strings {
    app_title: "Kontextmenü-Manager",

    tab_categories: "Kategorien",
    tab_filetypes: "Dateitypen",
    tab_programs: "Programme",
    tab_backups: "Sicherungen",

    cat_all_files: "Alle Dateien",
    cat_all_filesystem_objects: "Alle Dateisystemobjekte",
    cat_directory: "Ordner",
    cat_directory_background: "Ordner-Hintergrund",
    cat_folder: "Ordner und Shell-Namensraum",
    cat_desktop_background: "Desktop-Hintergrund",
    cat_drive: "Laufwerke",

    col_name: "Name",
    col_type: "Typ",
    col_location: "Ort",
    col_scope: "Bereich",
    col_command: "Befehl",
    col_flags: "Merkmale",

    btn_rescan: "Neu einlesen",
    btn_delete: "Löschen",
    btn_disable: "Ausblenden",
    btn_shift_only: "Nur mit Umschalt",
    btn_block: "Blockieren",
    btn_restore: "Wiederherstellen",
    btn_cancel: "Abbrechen",
    btn_execute: "Ausführen",
    btn_select_all: "Alle auswählen",
    btn_select_none: "Auswahl aufheben",

    badge_admin: "Administrator",
    badge_blocked: "blockiert",
    badge_shift: "nur mit Umschalt",
    badge_hidden: "ausgeblendet",
    badge_readonly: "schreibgeschützt",
    badge_system: "Systemkomponente",

    kind_verb: "Verb",
    kind_shellex: "COM-Handler",

    settings: "Einstellungen",
    language: "Sprache",
    theme: "Darstellung",
    theme_system: "System folgen",
    theme_light: "Hell",
    theme_dark: "Dunkel",

    search_hint: "Suchen in Name, Befehl und Pfad",
    filter_hide_empty: "Leere ausblenden",

    status_scanning: "Wird eingelesen …",
    status_ready: "Bereit",
    status_elevated: "Mit Administratorrechten",
    status_not_elevated: "Ohne Administratorrechte",
    msg_needs_admin: "Dieser Eintrag lässt sich nur mit Administratorrechten ändern.",
    msg_no_selection: "Nichts ausgewählt.",
    msg_confirm_delete: "Wirklich löschen? Vorher wird automatisch gesichert.",
    msg_backup_first: "Vor jeder Änderung wird gesichert.",
    msg_com_handler_note: "Der Text eines COM-Handlers entsteht erst zur Laufzeit und steht nicht in der Registry.",
    msg_restart_explorer: "Änderungen an COM-Handlern werden erst nach einem Neustart des Explorers sichtbar.",

    fmt_entries_found: "{} Einträge",
    fmt_scanning_location: "Lese {} …",
    fmt_selected_count: "{} ausgewählt",
    fmt_affects_other_types: "Gilt für alle Dateien. Löschen entfernt den Eintrag auch bei {} anderen Dateitypen.",
    fmt_backup_created: "Sicherung angelegt: {}",

    detail_title: "Details",
    detail_registry_path: "Registry-Pfad",
    detail_display_name: "Anzeigename",
    detail_raw_value: "Rohwert",
    detail_icon: "Symbol",
    detail_command: "Befehl",
    detail_clsid: "CLSID",
    detail_server: "Server-DLL",
    detail_position: "Position",
    detail_applies_to: "Gilt für",
    detail_nothing_selected: "Kein Eintrag ausgewählt.",

    editor_new: "\u{ff0b} Neu",
    editor_title: "Eigenen Eintrag anlegen",
    editor_category: "Kategorie",
    editor_display_name: "Anzeigename",
    editor_key_name: "Schl\u{fc}sselname",
    editor_command: "Befehl",
    editor_icon: "Symbol",
    editor_position: "Position",
    editor_visibility: "Sichtbarkeit",
    editor_extended: "nur mit gedr\u{fc}ckter Umschalttaste",
    editor_created_before: "Bereits mit diesem Werkzeug angelegt:",
    editor_create: "Anlegen",
    pos_default: "keine",
    pos_top: "oben",
    pos_bottom: "unten",
};

pub static EN: Strings = Strings {
    app_title: "Context Menu Manager",

    tab_categories: "Categories",
    tab_filetypes: "File Types",
    tab_programs: "Programs",
    tab_backups: "Backups",

    cat_all_files: "All Files",
    cat_all_filesystem_objects: "All Filesystem Objects",
    cat_directory: "Folder",
    cat_directory_background: "Folder Background",
    cat_folder: "Folder and Shell Namespace",
    cat_desktop_background: "Desktop Background",
    cat_drive: "Drives",

    col_name: "Name",
    col_type: "Type",
    col_location: "Location",
    col_scope: "Scope",
    col_command: "Command",
    col_flags: "Flags",

    btn_rescan: "Rescan",
    btn_delete: "Delete",
    btn_disable: "Hide",
    btn_shift_only: "Shift Only",
    btn_block: "Block",
    btn_restore: "Restore",
    btn_cancel: "Cancel",
    btn_execute: "Run",
    btn_select_all: "Select All",
    btn_select_none: "Deselect All",

    badge_admin: "Administrator",
    badge_blocked: "Blocked",
    badge_shift: "Shift Only",
    badge_hidden: "Hidden",
    badge_readonly: "Read-only",
    badge_system: "System Component",

    kind_verb: "Verb",
    kind_shellex: "COM Handler",

    settings: "Settings",
    language: "Language",
    theme: "Appearance",
    theme_system: "Follow system",
    theme_light: "Light",
    theme_dark: "Dark",

    search_hint: "Search by name, command, and path",
    filter_hide_empty: "Hide empty items",

    status_scanning: "Scanning …",
    status_ready: "Ready",
    status_elevated: "With administrator privileges",
    status_not_elevated: "Without administrator privileges",
    msg_needs_admin: "This entry can only be modified with administrator privileges.",
    msg_no_selection: "Nothing selected.",
    msg_confirm_delete: "Really delete? It will be backed up automatically first.",
    msg_backup_first: "A backup is created before every change.",
    msg_com_handler_note: "The text of a COM handler is generated at runtime and is not stored in the registry.",
    msg_restart_explorer: "Changes to COM handlers only take effect after restarting Explorer.",

    fmt_entries_found: "{} entries",
    fmt_scanning_location: "Scanning {} …",
    fmt_selected_count: "{} selected",
    fmt_affects_other_types: "Applies to all files. Deleting will also remove the entry for {} other file types.",
    fmt_backup_created: "Backup created: {}",

    detail_title: "Details",
    detail_registry_path: "Registry Path",
    detail_display_name: "Display Name",
    detail_raw_value: "Raw Value",
    detail_icon: "Icon",
    detail_command: "Command",
    detail_clsid: "CLSID",
    detail_server: "Server DLL",
    detail_position: "Position",
    detail_applies_to: "Applies to",
    detail_nothing_selected: "No entry selected.",

    editor_new: "\u{ff0b} New",
    editor_title: "Create your own entry",
    editor_category: "Category",
    editor_display_name: "Display name",
    editor_key_name: "Key name",
    editor_command: "Command",
    editor_icon: "Icon",
    editor_position: "Position",
    editor_visibility: "Visibility",
    editor_extended: "only while Shift is held",
    editor_created_before: "Already created with this tool:",
    editor_create: "Create",
    pos_default: "none",
    pos_top: "top",
    pos_bottom: "bottom",
};

/// The two tables side by side, for checks that must cover both.
///
/// The struct literal already forces every field to be filled, so what is left
/// to guard is content: an accidentally empty string, or a placeholder that
/// survived in one language and was dropped in the other — which would produce
/// a sentence missing its number.
#[cfg(test)]
fn field_pairs() -> Vec<(&'static str, &'static str, &'static str)> {
    macro_rules! pairs {
        ($($field:ident),* $(,)?) => {
            vec![$((stringify!($field), DE.$field, EN.$field)),*]
        };
    }

    pairs![
        app_title,
        tab_categories,
        tab_filetypes,
        tab_programs,
        tab_backups,
        cat_all_files,
        cat_all_filesystem_objects,
        cat_directory,
        cat_directory_background,
        cat_folder,
        cat_desktop_background,
        cat_drive,
        col_name,
        col_type,
        col_location,
        col_scope,
        col_command,
        col_flags,
        btn_rescan,
        btn_delete,
        btn_disable,
        btn_shift_only,
        btn_block,
        btn_restore,
        btn_cancel,
        btn_execute,
        btn_select_all,
        btn_select_none,
        badge_admin,
        badge_blocked,
        badge_shift,
        badge_hidden,
        badge_readonly,
        badge_system,
        kind_verb,
        kind_shellex,
        settings,
        language,
        theme,
        theme_system,
        theme_light,
        theme_dark,
        search_hint,
        filter_hide_empty,
        status_scanning,
        status_ready,
        status_elevated,
        status_not_elevated,
        msg_needs_admin,
        msg_no_selection,
        msg_confirm_delete,
        msg_backup_first,
        msg_com_handler_note,
        msg_restart_explorer,
        fmt_entries_found,
        fmt_scanning_location,
        fmt_selected_count,
        fmt_affects_other_types,
        fmt_backup_created,
        detail_title,
        detail_registry_path,
        detail_display_name,
        detail_raw_value,
        detail_icon,
        detail_command,
        detail_clsid,
        detail_server,
        detail_position,
        detail_applies_to,
        detail_nothing_selected,
        editor_new,
        editor_title,
        editor_category,
        editor_display_name,
        editor_key_name,
        editor_command,
        editor_icon,
        editor_position,
        editor_visibility,
        editor_extended,
        editor_created_before,
        editor_create,
        pos_default,
        pos_top,
        pos_bottom,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_string_is_empty_in_either_language() {
        for (field, de, en) in field_pairs() {
            assert!(!de.trim().is_empty(), "DE.{field} is empty");
            assert!(!en.trim().is_empty(), "EN.{field} is empty");
        }
    }

    #[test]
    fn placeholders_match_between_the_languages() {
        for (field, de, en) in field_pairs() {
            assert_eq!(
                de.matches("{}").count(),
                en.matches("{}").count(),
                "{field}: placeholder count differs, one language would lose its number"
            );
        }
    }

    #[test]
    fn only_fields_named_fmt_carry_placeholders() {
        for (field, de, en) in field_pairs() {
            let expected = field.starts_with("fmt_");
            assert_eq!(de.contains("{}"), expected, "DE.{field}");
            assert_eq!(en.contains("{}"), expected, "EN.{field}");
        }
    }

    #[test]
    fn the_two_tables_are_actually_different() {
        // Guards against a copy of the German table being pasted in as
        // English. Identical entries are legitimate for names like "CLSID",
        // so this checks the proportion rather than every field.
        let pairs = field_pairs();
        let identical = pairs.iter().filter(|(_, de, en)| de == en).count();
        assert!(
            identical * 4 < pairs.len(),
            "{identical} of {} fields are identical -- looks like an untranslated copy",
            pairs.len()
        );
    }
}
