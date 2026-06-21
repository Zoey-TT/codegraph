//! CodeGraph CLI internals.
//!
//! This small library exists primarily so that integration tests can inspect
//! and exercise registry logic without invoking the binary.

pub mod incremental;
pub mod registry;
