//! Windows Context Menu Manager.
//!
//! The binary is a thin shell around this library so that integration tests
//! can drive the registry code directly.

pub mod app;
pub mod cli;
pub mod console;
pub mod elevation;
pub mod i18n;
pub mod icons;
pub mod model;
pub mod program;
pub mod registry;
pub mod settings;
pub mod smoke;
pub mod synthetic;
pub mod theme;
