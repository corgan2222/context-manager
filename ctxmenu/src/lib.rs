//! Windows Context Menu Manager.
//!
//! The binary is a thin shell around this library so that integration tests
//! can drive the registry code directly.

pub mod cli;
pub mod model;
pub mod registry;
pub mod smoke;
