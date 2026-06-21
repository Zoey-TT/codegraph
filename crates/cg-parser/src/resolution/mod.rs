//! Name resolution and symbol lookup.
//!
//! Three-tier lookup system:
//! - Tier 1 (confidence 0.95): same-file symbol table
//! - Tier 2 (confidence 0.90): import scope (follow import chains)
//! - Tier 3 (confidence 0.50): global index (O(1) class/interface/callable lookup)

use std::collections::HashMap;

use cg_common::{CodeEdge, EdgeKind, NodeId};

pub mod cross_file;
pub mod resolve;

/// A symbol table for a single file.
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    /// Map from symbol name → node id (within the same file).
    locals: HashMap<String, NodeId>,
    /// Imports from this file: local alias → (original name, source path).
    imports: HashMap<String, (String, String)>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a local symbol.
    pub fn add_local(&mut self, name: impl Into<String>, id: NodeId) {
        self.locals.insert(name.into(), id);
    }

    /// Register an import.
    pub fn add_import(
        &mut self,
        local_name: impl Into<String>,
        original_name: String,
        source_path: String,
    ) {
        self.imports
            .insert(local_name.into(), (original_name, source_path));
    }

    /// Tier 1 lookup: same-file symbol.
    pub fn lookup_local(&self, name: &str) -> Option<NodeId> {
        self.locals.get(name).copied()
    }

    /// Tier 2 lookup: imported name.
    pub fn lookup_import(&self, name: &str) -> Option<&(String, String)> {
        self.imports.get(name)
    }

    /// All locals (for Tier 3 fallback or global indexing).
    pub fn locals(&self) -> &HashMap<String, NodeId> {
        &self.locals
    }
}

/// Resolve a call site within a single file's symbol table.
///
/// Returns an edge if the callee can be resolved (Tier 1 only for now).
pub fn resolve_call(caller_id: NodeId, callee_name: &str, table: &SymbolTable) -> Option<CodeEdge> {
    // Tier 1: same-file lookup
    if let Some(target_id) = table.lookup_local(callee_name) {
        return Some(CodeEdge::new(
            edge_id(caller_id, target_id, "calls"),
            caller_id,
            target_id,
            EdgeKind::Calls,
            0.95,
            "same-file",
        ));
    }
    None
}

/// Create IMPORTS edges from extracted import statements.
pub fn build_import_edges(
    file_id: NodeId,
    imports: &[crate::extractors::ExtractedImport],
) -> Vec<CodeEdge> {
    let mut edges = Vec::new();
    for imp in imports {
        // Create a deterministic NodeId for the import target
        let target_id = import_target_id(&imp.source);
        if imp.is_wildcard {
            edges.push(CodeEdge::new(
                edge_id(file_id, target_id, "imports-wildcard"),
                file_id,
                target_id,
                EdgeKind::Imports,
                0.80,
                format!("use {}::*", imp.source),
            ));
        } else {
            for name in &imp.names {
                let target_id = import_target_id(&format!("{}::{}", imp.source, name.local_name));
                edges.push(CodeEdge::new(
                    edge_id(file_id, target_id, "imports"),
                    file_id,
                    target_id,
                    EdgeKind::Imports,
                    0.90,
                    format!("use {}", imp.source),
                ));
            }
        }
    }
    edges
}

fn edge_id(a: NodeId, b: NodeId, label: &str) -> NodeId {
    use rustc_hash::FxHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    a.hash(&mut hasher);
    b.hash(&mut hasher);
    label.hash(&mut hasher);
    NodeId::new(hasher.finish())
}

fn import_target_id(path: &str) -> NodeId {
    use rustc_hash::FxHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    "import".hash(&mut hasher);
    path.hash(&mut hasher);
    NodeId::new(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cg_common::NodeId;

    #[test]
    fn symbol_table_lookup() {
        let mut table = SymbolTable::new();
        let id = NodeId::new(42);
        table.add_local("foo", id);
        assert_eq!(table.lookup_local("foo"), Some(id));
        assert_eq!(table.lookup_local("bar"), None);
    }

    #[test]
    fn build_import_edges_smoke() {
        let file_id = NodeId::new(1);
        let imports = vec![crate::extractors::ExtractedImport {
            source: "std::collections".to_string(),
            names: vec![crate::extractors::ImportedName {
                local_name: "HashMap".to_string(),
                original_name: None,
            }],
            range: tree_sitter::Range {
                start_byte: 0,
                end_byte: 20,
                start_point: tree_sitter::Point { row: 0, column: 0 },
                end_point: tree_sitter::Point { row: 0, column: 20 },
            },
            is_wildcard: false,
        }];
        let edges = build_import_edges(file_id, &imports);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Imports);
    }
}
