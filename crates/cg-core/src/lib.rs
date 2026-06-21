//! CodeGraph — In-memory knowledge graph and pipeline context.
//!
//! Provides `KnowledgeGraph`, a concurrent in-memory graph structure
//! used during pipeline execution and JSONL export.

use std::path::PathBuf;

use dashmap::DashMap;
use rustc_hash::FxHasher;
use std::hash::BuildHasherDefault;

use cg_common::{CodeEdge, CodeNode, EdgeKind, NodeId, NodeKind};

pub mod community;
pub mod incremental;
pub mod mro;
pub mod process;

/// Alias for a `DashMap` using FxHash.
pub type FxDashMap<K, V> = DashMap<K, V, BuildHasherDefault<FxHasher>>;

/// In-memory knowledge graph used during pipeline execution.
///
/// All fields use `DashMap` for concurrent access during parallel parsing.
#[derive(Debug, Clone)]
pub struct KnowledgeGraph {
    /// Node storage: NodeId → CodeNode
    pub nodes: FxDashMap<NodeId, CodeNode>,
    /// Outgoing edge adjacency list: NodeId → [CodeEdge]
    pub out_edges: FxDashMap<NodeId, Vec<CodeEdge>>,
    /// Incoming edge reverse index: NodeId → [NodeId] (source ids)
    pub in_edges: FxDashMap<NodeId, Vec<NodeId>>,
    /// Symbol index: (name, language) → [NodeId]
    pub symbol_index: FxDashMap<(String, Option<String>), Vec<NodeId>>,
    /// File path index: PathBuf → [NodeId]
    pub file_index: FxDashMap<PathBuf, Vec<NodeId>>,
}

impl KnowledgeGraph {
    /// Create a new empty knowledge graph.
    pub fn new() -> Self {
        Self {
            nodes: FxDashMap::default(),
            out_edges: FxDashMap::default(),
            in_edges: FxDashMap::default(),
            symbol_index: FxDashMap::default(),
            file_index: FxDashMap::default(),
        }
    }

    /// Insert a node and update all indexes.
    pub fn add_node(&self, node: CodeNode) {
        let id = node.id;
        let file_path = node.properties.file_path.clone();
        let name = node.properties.name.clone();
        let lang = node.properties.language.map(|l| format!("{:?}", l));

        self.nodes.insert(id, node);
        self.file_index.entry(file_path).or_default().push(id);
        self.symbol_index.entry((name, lang)).or_default().push(id);
    }

    /// Insert an edge and update adjacency lists.
    pub fn add_edge(&self, edge: CodeEdge) {
        let source = edge.source_id;
        let target = edge.target_id;

        self.out_edges.entry(source).or_default().push(edge);
        self.in_edges.entry(target).or_default().push(source);
    }

    /// Get a node by id.
    pub fn get_node(&self, id: &NodeId) -> Option<CodeNode> {
        self.nodes.get(id).map(|r| r.clone())
    }

    /// Get outgoing edges for a node.
    pub fn outgoing_edges(&self, id: &NodeId) -> Vec<CodeEdge> {
        self.out_edges
            .get(id)
            .map(|r| r.clone())
            .unwrap_or_default()
    }

    /// Get incoming edge sources for a node.
    pub fn incoming_sources(&self, id: &NodeId) -> Vec<NodeId> {
        self.in_edges.get(id).map(|r| r.clone()).unwrap_or_default()
    }

    /// Query nodes by kind.
    pub fn nodes_by_kind(&self, kind: NodeKind) -> Vec<CodeNode> {
        self.nodes
            .iter()
            .filter(|entry| entry.value().kind == kind)
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Query outgoing edges filtered by edge kind.
    pub fn outgoing_edges_by_kind(&self, id: &NodeId, edge_kind: EdgeKind) -> Vec<CodeEdge> {
        self.out_edges
            .get(id)
            .map(|edges| {
                edges
                    .iter()
                    .filter(|e| e.kind == edge_kind)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Remove a node and all associated edges.
    pub fn remove_node(&self, id: &NodeId) {
        // Remove from node store
        if let Some((_, node)) = self.nodes.remove(id) {
            // Remove from file index
            self.file_index
                .alter(&node.properties.file_path, |_, mut vec| {
                    vec.retain(|&nid| nid != *id);
                    vec
                });
            // Remove from symbol index
            let lang = node.properties.language.map(|l| format!("{:?}", l));
            self.symbol_index
                .alter(&(node.properties.name.clone(), lang), |_, mut vec| {
                    vec.retain(|&nid| nid != *id);
                    vec
                });
        }

        // Remove outgoing edges
        if let Some((_, edges)) = self.out_edges.remove(id) {
            for edge in edges {
                self.in_edges.alter(&edge.target_id, |_, mut vec| {
                    vec.retain(|&nid| nid != *id);
                    vec
                });
            }
        }

        // Remove incoming edge references
        if let Some((_, sources)) = self.in_edges.remove(id) {
            for source in sources {
                self.out_edges.alter(&source, |_, mut vec| {
                    vec.retain(|e| e.target_id != *id);
                    vec
                });
            }
        }
    }

    /// Total node count.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Total edge count (outgoing adjacency list entries).
    pub fn edge_count(&self) -> usize {
        self.out_edges.iter().map(|entry| entry.value().len()).sum()
    }

    /// Remove all outgoing edges of a specific kind from a node.
    pub fn remove_outgoing_edges_by_kind(&self, id: &NodeId, edge_kind: EdgeKind) {
        if let Some(mut edges) = self.out_edges.get_mut(id) {
            let removed_targets: Vec<NodeId> = edges
                .iter()
                .filter(|e| e.kind == edge_kind)
                .map(|e| e.target_id)
                .collect();
            edges.retain(|e| e.kind != edge_kind);
            drop(edges);
            for target in removed_targets {
                self.in_edges.alter(&target, |_, mut vec| {
                    vec.retain(|&sid| sid != *id);
                    vec
                });
            }
        }
    }
}

impl Default for KnowledgeGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Delta for incremental updates to the knowledge graph.
#[derive(Debug, Clone, Default)]
pub struct GraphDelta {
    pub nodes_to_add: Vec<CodeNode>,
    pub nodes_to_remove: Vec<NodeId>,
    pub edges_to_add: Vec<CodeEdge>,
    pub edges_to_remove: Vec<NodeId>, // edge ids
}

impl GraphDelta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes_to_add.is_empty()
            && self.nodes_to_remove.is_empty()
            && self.edges_to_add.is_empty()
            && self.edges_to_remove.is_empty()
    }
}

/// Apply a delta to the knowledge graph.
pub trait ApplyDelta {
    fn apply_delta(&self, delta: GraphDelta);
}

impl ApplyDelta for KnowledgeGraph {
    fn apply_delta(&self, delta: GraphDelta) {
        for node in delta.nodes_to_remove {
            self.remove_node(&node);
        }
        for node in delta.nodes_to_add {
            self.add_node(node);
        }
        for edge in delta.edges_to_add {
            self.add_edge(edge);
        }
        for edge_id in delta.edges_to_remove {
            // Remove edge by id from all adjacency lists
            self.out_edges.iter_mut().for_each(|mut entry| {
                entry.value_mut().retain(|e| e.id != edge_id);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cg_common::{NodeKind, NodeProperties};

    fn make_node(id: u64, name: &str, kind: NodeKind) -> CodeNode {
        CodeNode::new(NodeId::new(id), kind, NodeProperties::new(name, "/test.rs"))
    }

    #[test]
    fn add_and_get_node() {
        let graph = KnowledgeGraph::new();
        let node = make_node(1, "main", NodeKind::Function);
        graph.add_node(node.clone());
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.get_node(&NodeId::new(1)), Some(node));
    }

    #[test]
    fn add_edge() {
        let graph = KnowledgeGraph::new();
        let a = make_node(1, "caller", NodeKind::Function);
        let b = make_node(2, "callee", NodeKind::Function);
        graph.add_node(a);
        graph.add_node(b);

        let edge = CodeEdge::new(
            NodeId::new(100),
            NodeId::new(1),
            NodeId::new(2),
            EdgeKind::Calls,
            0.95,
            "direct call",
        );
        graph.add_edge(edge);

        assert_eq!(graph.edge_count(), 1);
        assert_eq!(graph.outgoing_edges(&NodeId::new(1)).len(), 1);
        assert_eq!(graph.incoming_sources(&NodeId::new(2)).len(), 1);
    }

    #[test]
    fn remove_node_cascades() {
        let graph = KnowledgeGraph::new();
        let a = make_node(1, "a", NodeKind::Function);
        let b = make_node(2, "b", NodeKind::Function);
        graph.add_node(a);
        graph.add_node(b);
        graph.add_edge(CodeEdge::new(
            NodeId::new(100),
            NodeId::new(1),
            NodeId::new(2),
            EdgeKind::Calls,
            0.95,
            "call",
        ));

        graph.remove_node(&NodeId::new(1));
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn apply_delta() {
        let graph = KnowledgeGraph::new();
        let node = make_node(1, "main", NodeKind::Function);
        let mut delta = GraphDelta::new();
        delta.nodes_to_add.push(node);
        graph.apply_delta(delta);
        assert_eq!(graph.node_count(), 1);
    }
}
