//! CodeGraph — File scanning, tree-sitter parsing, and symbol extraction.
//!
//! Pipeline phases for the minimal release:
//!   scan → structure → parse → crossFile → mro → communities → processes

pub mod extractors;
pub mod languages;
pub mod parser;
pub mod pipeline;
pub mod providers;
pub mod resolution;
pub mod scanner;
pub mod structure;

/// Re-export common types used throughout the parser.
pub use cg_common::{CaptureTag, Language, NodeKind};
