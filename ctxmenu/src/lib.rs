//! Windows Context Menu Manager.
//!
//! The binary is a thin shell around this library so that integration tests
//! can drive the registry code directly.

/// The version, from the one place it is written down.
///
/// `Cargo.toml` is the single source: the window title, `ctxmenu --version`
/// and the version resource of the `.exe` all derive from it at build time, so
/// there is no second number that can disagree with the first. `winresource`
/// picks up the same value on its own — the resource is not maintained by
/// hand either.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod app;
pub mod bilingual;
pub mod cli;
pub mod console;
pub mod elevation;
pub mod favourites;
pub mod filedialog;
pub mod i18n;
pub mod icons;
pub mod log;
pub mod model;
pub mod notify;
pub mod program;
pub mod registry;
pub mod service;
pub mod settings;
pub mod smoke;
pub mod synthetic;
pub mod theme;
pub mod update;
pub mod webtool;
