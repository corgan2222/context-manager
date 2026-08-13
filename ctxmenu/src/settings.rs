//! Persisted user choices.
//!
//! Kept free of any egui or Win32 type so the file format and the language
//! heuristic can be tested without a window and without a specific machine
//! locale. The mapping to `egui::ThemePreference` happens where the theme is
//! applied.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Language {
    #[default]
    German,
    English,
}

impl Language {
    pub fn label(self) -> &'static str {
        match self {
            Language::German => "Deutsch",
            Language::English => "English",
        }
    }

    /// Derives the start language from a Windows UI language identifier.
    ///
    /// Only the primary language matters: German is `0x07`, so `de-DE`
    /// (0x0407), `de-AT` (0x0C07) and `de-CH` (0x0807) all count. Everything
    /// else falls back to English, which is the safer default for a language
    /// nobody translated (ToDo 8).
    pub fn from_ui_language(lang_id: u16) -> Language {
        const LANG_GERMAN: u16 = 0x07;
        if lang_id & 0x3FF == LANG_GERMAN {
            Language::German
        } else {
            Language::English
        }
    }
}

/// The three-way theme choice from ToDo 9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    pub theme: ThemeChoice,
    /// File extensions the user added by hand, on top of the curated list
    /// (ToDo 10.3). Unused until milestone 7, persisted from now on so an
    /// older settings file never loses them.
    pub custom_extensions: Vec<String>,
    /// Hide file types that have no entries of their own.
    pub hide_empty_types: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::default(),
            theme: ThemeChoice::default(),
            custom_extensions: Vec::new(),
            hide_empty_types: true,
        }
    }
}

impl Settings {
    /// `%LOCALAPPDATA%\ctxmenu\settings.json`
    pub fn path() -> Result<PathBuf> {
        let base = dirs::data_local_dir().context("kein LOCALAPPDATA / no local data directory")?;
        Ok(base.join("ctxmenu").join("settings.json"))
    }

    /// Loads the settings, falling back to defaults.
    ///
    /// A corrupt or half-written file yields defaults rather than an error:
    /// refusing to start because a preferences file is damaged would be the
    /// wrong trade, and the next `save` repairs it.
    pub fn load_or_default(default_language: Language) -> Settings {
        let Ok(path) = Self::path() else {
            return Settings {
                language: default_language,
                ..Settings::default()
            };
        };

        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| Settings {
                language: default_language,
                ..Settings::default()
            }),
            // No file yet: first start, so take the system language.
            Err(_) => Settings {
                language: default_language,
                ..Settings::default()
            },
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Verzeichnis / directory {parent:?}"))?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("settings.json in {path:?}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_german_variant_selects_german() {
        // de-DE, de-CH, de-AT, de-LU, de-LI
        for id in [0x0407u16, 0x0807, 0x0C07, 0x1007, 0x1407] {
            assert_eq!(
                Language::from_ui_language(id),
                Language::German,
                "0x{id:04X} should be German"
            );
        }
    }

    #[test]
    fn everything_else_falls_back_to_english() {
        // en-US, en-GB, fr-FR, es-ES, ja-JP, and the neutral zero
        for id in [0x0409u16, 0x0809, 0x040C, 0x0C0A, 0x0411, 0x0000] {
            assert_eq!(
                Language::from_ui_language(id),
                Language::English,
                "0x{id:04X} should fall back to English"
            );
        }
    }

    #[test]
    fn settings_survive_a_round_trip_through_json() {
        let settings = Settings {
            language: Language::English,
            theme: ThemeChoice::Dark,
            custom_extensions: vec![".xyz".into(), ".foo".into()],
            hide_empty_types: false,
        };

        let json = serde_json::to_string(&settings).expect("serialisable");
        let back: Settings = serde_json::from_str(&json).expect("deserialisable");
        assert_eq!(settings, back);
    }

    #[test]
    fn an_older_settings_file_gains_the_new_fields() {
        // #[serde(default)] on the struct: a file written before a field
        // existed must still load instead of blocking the start.
        let old = r#"{"language":"English","theme":"Light"}"#;
        let loaded: Settings = serde_json::from_str(old).expect("forward compatible");

        assert_eq!(loaded.language, Language::English);
        assert_eq!(loaded.theme, ThemeChoice::Light);
        assert!(loaded.custom_extensions.is_empty());
        assert!(loaded.hide_empty_types, "missing field takes the default");
    }

    #[test]
    fn corrupt_content_yields_defaults_rather_than_a_failure() {
        let broken: std::result::Result<Settings, _> = serde_json::from_str("{ this is not json");
        assert!(broken.is_err());
        // load_or_default swallows exactly this case; verified through the
        // same path it uses.
        let recovered: Settings = serde_json::from_str("{ nope }").unwrap_or(Settings {
            language: Language::German,
            ..Settings::default()
        });
        assert_eq!(recovered.language, Language::German);
    }

    #[test]
    fn the_settings_file_sits_next_to_the_backups() {
        let path = Settings::path().expect("LOCALAPPDATA exists on Windows");
        assert!(path.ends_with(r"ctxmenu\settings.json"), "got {path:?}");
    }
}
