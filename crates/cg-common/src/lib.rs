//! CodeGraph — Shared types, constants, and utilities.
//!
//! This crate is the single source of truth for:
//! - Node and edge kind enumerations
//! - Language definitions
//! - Core node/edge data structures
//! - Confidence constants and import semantics

use std::path::PathBuf;
use std::str::FromStr;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

// ============================================================================
// Language
// ============================================================================

/// Supported programming languages for the minimal release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Language {
    JavaScript,
    TypeScript,
    Python,
    Java,
    C,
    Cpp,
    CSharp,
    Go,
    Rust,
}

impl Language {
    /// Infer language from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "js" | "mjs" | "cjs" => Some(Language::JavaScript),
            "ts" | "tsx" | "mts" | "cts" => Some(Language::TypeScript),
            "py" | "pyi" => Some(Language::Python),
            "java" => Some(Language::Java),
            "c" => Some(Language::C),
            "cpp" | "cc" | "cxx" | "hpp" | "h" => Some(Language::Cpp),
            "cs" => Some(Language::CSharp),
            "go" => Some(Language::Go),
            "rs" => Some(Language::Rust),
            _ => None,
        }
    }

    /// Canonical file extensions for this language.
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            Language::JavaScript => &["js", "mjs", "cjs"],
            Language::TypeScript => &["ts", "tsx", "mts", "cts"],
            Language::Python => &["py", "pyi"],
            Language::Java => &["java"],
            Language::C => &["c"],
            Language::Cpp => &["cpp", "cc", "cxx", "hpp", "h"],
            Language::CSharp => &["cs"],
            Language::Go => &["go"],
            Language::Rust => &["rs"],
        }
    }
}

/// Error returned when parsing a [`Language`] from a string fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseLanguageError;

impl std::fmt::Display for ParseLanguageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown language identifier")
    }
}

impl std::error::Error for ParseLanguageError {}

impl FromStr for Language {
    type Err = ParseLanguageError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "JavaScript" => Ok(Language::JavaScript),
            "TypeScript" => Ok(Language::TypeScript),
            "Python" => Ok(Language::Python),
            "Java" => Ok(Language::Java),
            "C" => Ok(Language::C),
            "Cpp" => Ok(Language::Cpp),
            "CSharp" => Ok(Language::CSharp),
            "Go" => Ok(Language::Go),
            "Rust" => Ok(Language::Rust),
            _ => Err(ParseLanguageError),
        }
    }
}

// ============================================================================
// NodeId
// ============================================================================

/// Newtype wrapper around `u64` for node identifiers.
/// Uses FxHash for fast hashing in hash maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u64);

impl NodeId {
    /// Create a new NodeId from a raw u64 value.
    pub const fn new(id: u64) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================================
// NodeKind
// ============================================================================

/// All possible node kinds in the knowledge graph.
///
/// Replaces the string-literal `NodeLabel` from the TypeScript version
/// with compile-time exhaustive checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum NodeKind {
    Project,
    Package,
    Module,
    Folder,
    File,
    Class,
    Function,
    Method,
    Variable,
    Interface,
    Enum,
    Decorator,
    Import,
    Type,
    CodeElement,
    Community,
    Process,
    // Multi-language node types
    Struct,
    Macro,
    Typedef,
    Union,
    Namespace,
    Trait,
    Impl,
    TypeAlias,
    Const,
    Static,
    Property,
    Record,
    Delegate,
    Annotation,
    Constructor,
    Template,
    Section,
    Route,
    Tool,
}

impl NodeKind {
    /// String representation used for database labels and debugging.
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeKind::Project => "Project",
            NodeKind::Package => "Package",
            NodeKind::Module => "Module",
            NodeKind::Folder => "Folder",
            NodeKind::File => "File",
            NodeKind::Class => "Class",
            NodeKind::Function => "Function",
            NodeKind::Method => "Method",
            NodeKind::Variable => "Variable",
            NodeKind::Interface => "Interface",
            NodeKind::Enum => "Enum",
            NodeKind::Decorator => "Decorator",
            NodeKind::Import => "Import",
            NodeKind::Type => "Type",
            NodeKind::CodeElement => "CodeElement",
            NodeKind::Community => "Community",
            NodeKind::Process => "Process",
            NodeKind::Struct => "Struct",
            NodeKind::Macro => "Macro",
            NodeKind::Typedef => "Typedef",
            NodeKind::Union => "Union",
            NodeKind::Namespace => "Namespace",
            NodeKind::Trait => "Trait",
            NodeKind::Impl => "Impl",
            NodeKind::TypeAlias => "TypeAlias",
            NodeKind::Const => "Const",
            NodeKind::Static => "Static",
            NodeKind::Property => "Property",
            NodeKind::Record => "Record",
            NodeKind::Delegate => "Delegate",
            NodeKind::Annotation => "Annotation",
            NodeKind::Constructor => "Constructor",
            NodeKind::Template => "Template",
            NodeKind::Section => "Section",
            NodeKind::Route => "Route",
            NodeKind::Tool => "Tool",
        }
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================================
// EdgeKind
// ============================================================================

/// All possible edge (relationship) kinds in the knowledge graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EdgeKind {
    Contains,
    Calls,
    Inherits,
    MethodOverrides,
    MethodImplements,
    Imports,
    Uses,
    Defines,
    Decorates,
    Implements,
    Extends,
    HasMethod,
    HasProperty,
    Accesses,
    MemberOf,
    StepInProcess,
    HandlesRoute,
    Fetches,
    HandlesTool,
    EntryPointOf,
    Wraps,
    Queries,
}

impl EdgeKind {
    /// String representation used for database relationship types.
    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeKind::Contains => "CONTAINS",
            EdgeKind::Calls => "CALLS",
            EdgeKind::Inherits => "INHERITS",
            EdgeKind::MethodOverrides => "METHOD_OVERRIDES",
            EdgeKind::MethodImplements => "METHOD_IMPLEMENTS",
            EdgeKind::Imports => "IMPORTS",
            EdgeKind::Uses => "USES",
            EdgeKind::Defines => "DEFINES",
            EdgeKind::Decorates => "DECORATES",
            EdgeKind::Implements => "IMPLEMENTS",
            EdgeKind::Extends => "EXTENDS",
            EdgeKind::HasMethod => "HAS_METHOD",
            EdgeKind::HasProperty => "HAS_PROPERTY",
            EdgeKind::Accesses => "ACCESSES",
            EdgeKind::MemberOf => "MEMBER_OF",
            EdgeKind::StepInProcess => "STEP_IN_PROCESS",
            EdgeKind::HandlesRoute => "HANDLES_ROUTE",
            EdgeKind::Fetches => "FETCHES",
            EdgeKind::HandlesTool => "HANDLES_TOOL",
            EdgeKind::EntryPointOf => "ENTRY_POINT_OF",
            EdgeKind::Wraps => "WRAPS",
            EdgeKind::Queries => "QUERIES",
        }
    }
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================================
// Confidence
// ============================================================================

/// Confidence score for edges (0.0 ..= 1.0).
pub type Confidence = f64;

/// Confidence constants for the three-tier name lookup system.
pub mod confidence {
    use super::Confidence;

    /// Same-file symbol lookup (highest confidence).
    pub const SAME_FILE: Confidence = 0.95;

    /// Import-scope symbol lookup.
    pub const IMPORT_SCOPE: Confidence = 0.90;

    /// Global symbol lookup (lowest confidence).
    pub const GLOBAL: Confidence = 0.50;
}

// ============================================================================
// ImportSemantics
// ============================================================================

/// How a language resolves imported names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ImportSemantics {
    /// `import { X } from "mod"` — named imports.
    Named,
    /// `import * as mod from "mod"` — all exports available via namespace.
    WildcardLeaf,
    /// `#include "header.h"` — transitive inclusion chain.
    WildcardTransitive,
    /// `namespace alias` or `import pkg as alias`.
    Namespace,
}

// ============================================================================
// MroStrategy
// ============================================================================

/// Method Resolution Order strategy for a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum MroStrategy {
    /// C3 linearization (Python, etc.).
    C3,
    /// First declared match wins (Java, C# single inheritance).
    FirstWins,
    /// Ruby mixin semantics (depth-first + monotonicity check).
    RubyMixin,
    /// No inheritance / not applicable.
    None,
}

// ============================================================================
// CodeNode
// ============================================================================

/// Properties that can be attached to any node.
///
/// Using a flat `FxHashMap` allows forward-compatible extensibility
/// while keeping the core struct serializable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeProperties {
    pub name: String,
    pub file_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_exported: Option<bool>,
    /// Flat map for extensible attributes (community, process, method, route, etc.).
    #[serde(flatten)]
    pub extras: FxHashMap<String, serde_json::Value>,
}

impl NodeProperties {
    pub fn new(name: impl Into<String>, file_path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            file_path: file_path.into(),
            start_line: None,
            end_line: None,
            language: None,
            is_exported: None,
            extras: FxHashMap::default(),
        }
    }
}

/// Unified node representation in the knowledge graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CodeNode {
    pub id: NodeId,
    pub kind: NodeKind,
    pub properties: NodeProperties,
}

impl CodeNode {
    pub fn new(id: NodeId, kind: NodeKind, properties: NodeProperties) -> Self {
        Self {
            id,
            kind,
            properties,
        }
    }
}

// ============================================================================
// CodeEdge
// ============================================================================

/// Evidence trace for edges emitted by the scope-based resolution pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeEvidence {
    pub kind: String,
    pub weight: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Unified edge representation in the knowledge graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CodeEdge {
    pub id: NodeId,
    pub source_id: NodeId,
    pub target_id: NodeId,
    pub kind: EdgeKind,
    pub confidence: Confidence,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub evidence: Vec<EdgeEvidence>,
}

impl CodeEdge {
    pub fn new(
        id: NodeId,
        source_id: NodeId,
        target_id: NodeId,
        kind: EdgeKind,
        confidence: Confidence,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            id,
            source_id,
            target_id,
            kind,
            confidence,
            reason: reason.into(),
            step: None,
            evidence: Vec::new(),
        }
    }
}

// ============================================================================
// CaptureTag
// ============================================================================

/// Unified capture tags for tree-sitter query results.
/// Each language maps its tree-sitter captures to this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CaptureTag {
    DefinitionClass,
    DefinitionFunction,
    DefinitionMethod,
    DefinitionInterface,
    DefinitionStruct,
    DefinitionEnum,
    DefinitionTrait,
    DefinitionImpl,
    DefinitionVariable,
    DefinitionConst,
    DefinitionStatic,
    DefinitionTypeAlias,
    DefinitionMacro,
    DefinitionModule,
    DefinitionNamespace,
    DefinitionProperty,
    DefinitionField,
    DefinitionConstructor,
    CallName,
    CallMethod,
    CallConstructor,
    ImportSource,
    ImportName,
    ImportAlias,
    HeritageExtends,
    HeritageImplements,
    HeritageIncludes,
    ReferenceIdentifier,
    TypeIdentifier,
    FieldName,
    ParameterName,
    AnnotationName,
    RoutePath,
    RouteMethod,
    RouteHandler,
    ToolName,
    ToolHandler,
}

// ============================================================================
// Pipeline types
// ============================================================================

/// Pipeline execution phases (for progress reporting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PipelinePhase {
    Idle,
    Extracting,
    Structure,
    Parsing,
    Imports,
    Calls,
    Heritage,
    Communities,
    Processes,
    Enriching,
    Complete,
    Error,
}

/// Progress update sent during pipeline execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineProgress {
    pub phase: PipelinePhase,
    pub percent: f64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<PipelineStats>,
}

/// Snapshot statistics for progress reporting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineStats {
    pub files_processed: usize,
    pub total_files: usize,
    pub nodes_created: usize,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_from_extension() {
        assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("unknown"), None);
    }

    #[test]
    fn node_kind_roundtrip() {
        assert_eq!(NodeKind::Function.as_str(), "Function");
        assert_eq!(NodeKind::Struct.as_str(), "Struct");
    }

    #[test]
    fn edge_kind_roundtrip() {
        assert_eq!(EdgeKind::Calls.as_str(), "CALLS");
        assert_eq!(EdgeKind::Imports.as_str(), "IMPORTS");
    }

    #[test]
    fn confidence_constants() {
        assert_eq!(confidence::SAME_FILE, 0.95);
        assert_eq!(confidence::IMPORT_SCOPE, 0.90);
        assert_eq!(confidence::GLOBAL, 0.50);
    }

    #[test]
    fn node_id_display() {
        let id = NodeId::new(42);
        assert_eq!(format!("{}", id), "42");
    }

    #[test]
    fn code_node_serialization() {
        let node = CodeNode::new(
            NodeId::new(1),
            NodeKind::Function,
            NodeProperties::new("main", "/src/main.rs"),
        );
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("Function"));
        assert!(json.contains("main"));
    }
}
