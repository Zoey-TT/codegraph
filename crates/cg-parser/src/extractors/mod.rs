//! Symbol extraction traits and unified types.

use cg_common::{Language, NodeId, NodeKind};

/// A symbol extracted from AST.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedSymbol {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    pub range: tree_sitter::Range,
    pub parent_id: Option<NodeId>,
    pub extra: std::collections::HashMap<String, String>,
}

/// A call site extracted from AST.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedCall {
    pub caller_id: NodeId,
    pub callee_name: String,
    pub call_form: CallForm,
    pub range: tree_sitter::Range,
    pub receiver_name: Option<String>,
    pub argument_count: usize,
}

/// Form of a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallForm {
    Free,
    Member,
    Constructor,
}

/// An import statement extracted from AST.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedImport {
    pub source: String,
    pub names: Vec<ImportedName>,
    pub range: tree_sitter::Range,
    pub is_wildcard: bool,
}

/// A single imported name.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedName {
    pub local_name: String,
    pub original_name: Option<String>,
}

/// A heritage relationship (extends/implements/includes).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedHeritage {
    pub child_id: NodeId,
    pub parent_name: String,
    pub kind: HeritageKind,
    pub range: tree_sitter::Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeritageKind {
    Extends,
    Implements,
    Includes,
}

// ============================================================================
// Extractor traits
// ============================================================================

/// Extracts symbol definitions from AST.
pub trait SymbolExtractor: Send + Sync {
    fn language(&self) -> Language;
    fn extract(&self, parsed: &crate::parser::ParsedFile) -> Vec<ExtractedSymbol>;
}

/// Extracts call sites from AST.
pub trait CallExtractor: Send + Sync {
    fn language(&self) -> Language;
    fn extract(&self, parsed: &crate::parser::ParsedFile) -> Vec<ExtractedCall>;
}

/// Extracts import statements from AST.
pub trait ImportExtractor: Send + Sync {
    fn language(&self) -> Language;
    fn extract(&self, parsed: &crate::parser::ParsedFile) -> Vec<ExtractedImport>;
}

/// Extracts heritage relationships from AST.
pub trait HeritageExtractor: Send + Sync {
    fn language(&self) -> Language;
    fn extract(&self, parsed: &crate::parser::ParsedFile) -> Vec<ExtractedHeritage>;
}
