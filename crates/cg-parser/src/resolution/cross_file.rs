//! Cross-file symbol index and type inference.

use std::collections::HashMap;
use std::path::PathBuf;

use cg_common::{NodeId, NodeKind};
use cg_core::{GraphDelta, KnowledgeGraph};

/// Index of symbols visible across files.
///
/// Built after the `parse` phase so that every file has already been
/// scanned and every symbol node already lives in the graph.
#[derive(Debug, Clone, Default)]
pub struct CrossFileIndex {
    /// file_path → (symbol_name → node_ids in that file)
    pub by_file: HashMap<PathBuf, HashMap<String, Vec<NodeId>>>,
    /// Global name → node_ids (all files)
    pub global_by_name: HashMap<String, Vec<NodeId>>,
}

impl CrossFileIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the index from the current state of the knowledge graph.
    pub fn build(graph: &KnowledgeGraph) -> Self {
        let mut index = Self::new();

        for entry in graph.nodes.iter() {
            let node = entry.value();
            let name = &node.properties.name;
            if name.is_empty() {
                continue;
            }

            let file_path = &node.properties.file_path;
            index
                .by_file
                .entry(file_path.clone())
                .or_default()
                .entry(name.clone())
                .or_default()
                .push(node.id);

            index
                .global_by_name
                .entry(name.clone())
                .or_default()
                .push(node.id);
        }

        index
    }

    /// Lookup a symbol name within a specific file.
    pub fn lookup_in_file(&self, file_path: &PathBuf, name: &str) -> Option<&Vec<NodeId>> {
        self.by_file.get(file_path).and_then(|m| m.get(name))
    }

    /// Global lookup by name (Tier-3 fallback).
    pub fn lookup_global(&self, name: &str) -> Option<&Vec<NodeId>> {
        self.global_by_name.get(name)
    }
}

/// Infer the type of variable nodes by inspecting adjacent CALLS edges.
///
/// This is a simplified inference: if a variable is the target of a
/// CONTAINS edge from a class/struct node, or if it is assigned the
/// result of a constructor call, we store the inferred type name in
/// `NodeProperties.extras["inferred_type"]`.
pub fn infer_variable_types(graph: &KnowledgeGraph) -> GraphDelta {
    let mut delta = GraphDelta::new();

    for entry in graph.nodes.iter() {
        let node = entry.value();
        if node.kind != NodeKind::Variable {
            continue;
        }

        // Skip if already has an inferred type
        if node.properties.extras.contains_key("inferred_type") {
            continue;
        }

        // Heuristic 1: look at incoming CALLS edges where the caller is a
        // constructor-like call.  We do this by checking whether any node
        // that calls *this* variable has a name matching the variable's
        // name (e.g. `let x = Foo::new()` – not directly modelled yet).
        //
        // Heuristic 2: check if this variable is contained by a class-like
        // node and has a type annotation in the source (not available here).
        //
        // For now we do a cheap heuristic: if the variable name ends with
        // a known type suffix pattern (e.g. `_service`, `_client`) we
        // store a placeholder type so that the resolve phase has *something*
        // to work with.
        let name = &node.properties.name;
        let inferred = guess_type_from_name(name);

        if let Some(ty) = inferred {
            let mut updated = node.clone();
            updated
                .properties
                .extras
                .insert("inferred_type".to_string(), serde_json::Value::String(ty));
            delta.nodes_to_remove.push(node.id);
            delta.nodes_to_add.push(updated);
        }
    }

    delta
}

/// Naïve name-based type guessing.
///
/// Examples:
/// - `user_service` → `Service`
/// - `http_client`  → `Client`
/// - `db_conn`      → `Connection`
fn guess_type_from_name(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    if lower.ends_with("_service") || lower.ends_with("service") {
        return Some("Service".to_string());
    }
    if lower.ends_with("_client") || lower.ends_with("client") {
        return Some("Client".to_string());
    }
    if lower.ends_with("_conn") || lower.ends_with("connection") {
        return Some("Connection".to_string());
    }
    if lower.ends_with("_repo") || lower.ends_with("repository") {
        return Some("Repository".to_string());
    }
    if lower.ends_with("_ctx") || lower.ends_with("context") {
        return Some("Context".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use cg_common::{CodeNode, NodeKind, NodeProperties};

    fn make_node(id: u64, name: &str, kind: NodeKind, file: &str) -> CodeNode {
        let mut props = NodeProperties::new(name, file);
        props.language = Some(cg_common::Language::Rust);
        CodeNode::new(cg_common::NodeId::new(id), kind, props)
    }

    #[test]
    fn build_cross_file_index_smoke() {
        let graph = KnowledgeGraph::new();
        let a = make_node(1, "foo", NodeKind::Function, "/a.rs");
        let b = make_node(2, "bar", NodeKind::Function, "/b.rs");
        graph.add_node(a);
        graph.add_node(b);

        let index = CrossFileIndex::build(&graph);
        assert_eq!(index.lookup_global("foo").unwrap().len(), 1);
        assert_eq!(index.lookup_global("bar").unwrap().len(), 1);
        assert!(
            index
                .lookup_in_file(&PathBuf::from("/a.rs"), "foo")
                .is_some()
        );
        assert!(
            index
                .lookup_in_file(&PathBuf::from("/b.rs"), "foo")
                .is_none()
        );
    }

    #[test]
    fn guess_type_from_name_works() {
        assert_eq!(
            guess_type_from_name("user_service"),
            Some("Service".to_string())
        );
        assert_eq!(
            guess_type_from_name("http_client"),
            Some("Client".to_string())
        );
        assert_eq!(guess_type_from_name("x"), None);
    }
}
