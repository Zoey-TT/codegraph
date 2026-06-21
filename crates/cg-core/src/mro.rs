//! Method Resolution Order (MRO) algorithms.
//!
//! Supports C3 linearization, first-wins (DFS), and Ruby-mixin strategies.

use std::collections::{HashMap, HashSet};

use cg_common::{EdgeKind, NodeId, NodeKind};

use crate::KnowledgeGraph;

#[derive(Debug, Clone, thiserror::Error)]
pub enum MroError {
    #[error("C3 linearization failed: inconsistent hierarchy")]
    InconsistentHierarchy,
    #[error("Cycle detected in inheritance graph")]
    Cycle,
}

/// Cached MRO results for all class-like nodes.
#[derive(Debug, Clone, Default)]
pub struct MroCache {
    /// node_id → MRO list (includes the node itself as the first element)
    pub mro: HashMap<NodeId, Vec<NodeId>>,
}

impl MroCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, node_id: NodeId) -> Option<&Vec<NodeId>> {
        self.mro.get(&node_id)
    }
}

/// Compute MRO for every class-like node in the graph.
///
/// Class-like kinds: Class, Struct, Trait, Interface.
pub fn compute_mro(graph: &KnowledgeGraph, strategy: cg_common::MroStrategy) -> MroCache {
    let mut cache = MroCache::new();

    let class_like = [
        NodeKind::Class,
        NodeKind::Struct,
        NodeKind::Trait,
        NodeKind::Interface,
    ];

    for kind in &class_like {
        for node in graph.nodes_by_kind(*kind) {
            let mro = match strategy {
                cg_common::MroStrategy::C3 => match c3_linearize(graph, node.id, &mut cache) {
                    Ok(m) => m,
                    Err(_) => first_wins_mro(graph, node.id, &mut cache),
                },
                cg_common::MroStrategy::FirstWins => first_wins_mro(graph, node.id, &mut cache),
                cg_common::MroStrategy::RubyMixin => {
                    match ruby_mixin_mro(graph, node.id, &mut cache) {
                        Ok(m) => m,
                        Err(_) => first_wins_mro(graph, node.id, &mut cache),
                    }
                }
                cg_common::MroStrategy::None => vec![node.id],
                _ => first_wins_mro(graph, node.id, &mut cache),
            };
            cache.mro.insert(node.id, mro);
        }
    }

    cache
}

// ============================================================================
// C3 Linearization
// ============================================================================

/// C3 linearization (Python-style MRO).
///
/// Returns a list where the first element is `class_id` itself,
/// followed by its parents in MRO order.
pub fn c3_linearize(
    graph: &KnowledgeGraph,
    class_id: NodeId,
    cache: &mut MroCache,
) -> Result<Vec<NodeId>, MroError> {
    if let Some(mro) = cache.mro.get(&class_id) {
        return Ok(mro.clone());
    }

    let direct = direct_parents(graph, class_id);

    if direct.is_empty() {
        cache.mro.insert(class_id, vec![class_id]);
        return Ok(vec![class_id]);
    }

    // Build the list of sequences to merge:
    // [MRO(P1), MRO(P2), ..., [P1, P2, ...]]
    let mut sequences: Vec<Vec<NodeId>> = Vec::new();
    for &parent_id in &direct {
        let parent_mro = c3_linearize(graph, parent_id, cache)?;
        sequences.push(parent_mro);
    }
    sequences.push(direct);

    let merged = c3_merge(sequences)?;
    let mut result = vec![class_id];
    result.extend(merged);
    cache.mro.insert(class_id, result.clone());
    Ok(result)
}

fn c3_merge(mut sequences: Vec<Vec<NodeId>>) -> Result<Vec<NodeId>, MroError> {
    let mut result = Vec::new();

    loop {
        sequences.retain(|s| !s.is_empty());
        if sequences.is_empty() {
            break;
        }

        // Find a head that does not appear in any tail.
        let mut chosen = None;
        'outer: for seq in &sequences {
            let head = seq[0];
            for other in &sequences {
                if other.len() > 1 && other[1..].contains(&head) {
                    continue 'outer;
                }
            }
            chosen = Some(head);
            break;
        }

        let head = chosen.ok_or(MroError::InconsistentHierarchy)?;

        for seq in &mut sequences {
            if seq.first() == Some(&head) {
                seq.remove(0);
            }
        }
        result.push(head);
    }

    Ok(result)
}

// ============================================================================
// First-wins (DFS pre-order with dedup)
// ============================================================================

pub fn first_wins_mro(
    graph: &KnowledgeGraph,
    class_id: NodeId,
    cache: &mut MroCache,
) -> Vec<NodeId> {
    if let Some(mro) = cache.mro.get(&class_id) {
        return mro.clone();
    }

    let mut result = vec![class_id];
    let mut visited = HashSet::new();
    visited.insert(class_id);

    fn dfs(
        graph: &KnowledgeGraph,
        node_id: NodeId,
        result: &mut Vec<NodeId>,
        visited: &mut HashSet<NodeId>,
    ) {
        for parent_id in direct_parents(graph, node_id) {
            if visited.insert(parent_id) {
                result.push(parent_id);
                dfs(graph, parent_id, result, visited);
            }
        }
    }

    dfs(graph, class_id, &mut result, &mut visited);
    cache.mro.insert(class_id, result.clone());
    result
}

// ============================================================================
// Ruby-mixin (depth-first + monotonicity check)
// ============================================================================

pub fn ruby_mixin_mro(
    graph: &KnowledgeGraph,
    class_id: NodeId,
    cache: &mut MroCache,
) -> Result<Vec<NodeId>, MroError> {
    if let Some(mro) = cache.mro.get(&class_id) {
        return Ok(mro.clone());
    }

    let mut result = vec![class_id];
    let mut visited = HashSet::new();
    visited.insert(class_id);

    fn dfs(
        graph: &KnowledgeGraph,
        node_id: NodeId,
        result: &mut Vec<NodeId>,
        visited: &mut HashSet<NodeId>,
    ) -> Result<(), MroError> {
        for parent_id in direct_parents(graph, node_id) {
            if !visited.insert(parent_id) {
                // In Ruby, re-visiting a parent is an error (monotonicity violation)
                return Err(MroError::Cycle);
            }
            result.push(parent_id);
            dfs(graph, parent_id, result, visited)?;
        }
        Ok(())
    }

    dfs(graph, class_id, &mut result, &mut visited)?;
    cache.mro.insert(class_id, result.clone());
    Ok(result)
}

// ============================================================================
// Helpers
// ============================================================================

/// Collect direct parent nodes via EXTENDS, IMPLEMENTS, and INHERITS edges.
fn direct_parents(graph: &KnowledgeGraph, class_id: NodeId) -> Vec<NodeId> {
    let mut parents = Vec::new();
    for kind in [EdgeKind::Extends, EdgeKind::Implements, EdgeKind::Inherits] {
        for edge in graph.outgoing_edges_by_kind(&class_id, kind) {
            if !parents.contains(&edge.target_id) {
                parents.push(edge.target_id);
            }
        }
    }
    parents
}

/// Look up a method by name in the MRO of a given class.
///
/// Checks `HasMethod` edges first, then `Contains` edges for Method nodes.
pub fn find_method_in_mro(
    graph: &KnowledgeGraph,
    cache: &MroCache,
    class_id: NodeId,
    method_name: &str,
) -> Option<NodeId> {
    let mro = cache.get(class_id)?;

    for &type_id in mro {
        // Check HasMethod edges
        for edge in graph.outgoing_edges_by_kind(&type_id, EdgeKind::HasMethod) {
            if let Some(node) = graph.get_node(&edge.target_id)
                && node.properties.name == method_name
            {
                return Some(edge.target_id);
            }
        }

        // Check Contains edges for Method/Function children
        for edge in graph.outgoing_edges_by_kind(&type_id, EdgeKind::Contains) {
            if let Some(node) = graph.get_node(&edge.target_id)
                && matches!(node.kind, NodeKind::Method | NodeKind::Function)
                && node.properties.name == method_name
            {
                return Some(edge.target_id);
            }
        }
    }

    None
}

/// Generate METHOD_OVERRIDES and METHOD_IMPLEMENTS edges from MRO data.
///
/// For each class/struct, walk its MRO. If a method in the class has the
/// same name as a method in a parent class, emit METHOD_OVERRIDES.
/// If the parent is a Trait/Interface, emit METHOD_IMPLEMENTS instead.
pub fn emit_override_edges(graph: &KnowledgeGraph, cache: &MroCache) -> Vec<cg_common::CodeEdge> {
    use cg_common::{CodeEdge, NodeKind};
    use rustc_hash::FxHasher;
    use std::hash::{Hash, Hasher};

    let mut edges = Vec::new();

    for (&class_id, mro) in &cache.mro {
        if mro.len() <= 1 {
            continue;
        }

        let class_methods: Vec<(String, NodeId)> = graph
            .outgoing_edges_by_kind(&class_id, EdgeKind::Contains)
            .into_iter()
            .filter_map(|e| graph.get_node(&e.target_id))
            .filter(|n| matches!(n.kind, NodeKind::Method | NodeKind::Function))
            .map(|n| (n.properties.name.clone(), n.id))
            .collect();

        for (method_name, method_id) in &class_methods {
            for &parent_id in &mro[1..] {
                if let Some(parent_method) =
                    find_method_in_mro(graph, cache, parent_id, method_name)
                {
                    let mut hasher = FxHasher::default();
                    method_id.hash(&mut hasher);
                    parent_method.hash(&mut hasher);
                    "override".hash(&mut hasher);
                    let edge_id = NodeId::new(hasher.finish());

                    let parent_node = graph.get_node(&parent_id);
                    let is_interface = parent_node
                        .map(|n| matches!(n.kind, NodeKind::Trait | NodeKind::Interface))
                        .unwrap_or(false);

                    let (kind, reason) = if is_interface {
                        (EdgeKind::MethodImplements, "interface implementation")
                    } else {
                        (EdgeKind::MethodOverrides, "method override")
                    };

                    edges.push(CodeEdge::new(
                        edge_id,
                        *method_id,
                        parent_method,
                        kind,
                        0.85,
                        reason,
                    ));
                    break; // Only link to the first match in MRO
                }
            }
        }
    }

    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KnowledgeGraph;
    use cg_common::{CodeEdge, CodeNode, NodeKind, NodeProperties};

    fn make_node(id: u64, name: &str, kind: NodeKind) -> CodeNode {
        CodeNode::new(
            cg_common::NodeId::new(id),
            kind,
            NodeProperties::new(name, "/test.rs"),
        )
    }

    fn make_edge(id: u64, source: u64, target: u64, kind: EdgeKind) -> CodeEdge {
        CodeEdge::new(
            cg_common::NodeId::new(id),
            cg_common::NodeId::new(source),
            cg_common::NodeId::new(target),
            kind,
            1.0,
            "test",
        )
    }

    #[test]
    fn c3_diamond() {
        //   A
        //  / \
        // B   C
        //  \ /
        //   D
        let graph = KnowledgeGraph::new();
        let a = make_node(1, "A", NodeKind::Class);
        let b = make_node(2, "B", NodeKind::Class);
        let c = make_node(3, "C", NodeKind::Class);
        let d = make_node(4, "D", NodeKind::Class);
        graph.add_node(a.clone());
        graph.add_node(b.clone());
        graph.add_node(c.clone());
        graph.add_node(d.clone());

        graph.add_edge(make_edge(10, 2, 1, EdgeKind::Extends)); // B -> A
        graph.add_edge(make_edge(11, 3, 1, EdgeKind::Extends)); // C -> A
        graph.add_edge(make_edge(12, 4, 2, EdgeKind::Extends)); // D -> B
        graph.add_edge(make_edge(13, 4, 3, EdgeKind::Extends)); // D -> C

        let mut cache = MroCache::new();
        let mro = c3_linearize(&graph, d.id, &mut cache).unwrap();
        let ids: Vec<u64> = mro.iter().map(|n| n.0).collect();
        assert_eq!(ids, vec![4, 2, 3, 1]);
    }

    #[test]
    fn first_wins_simple() {
        let graph = KnowledgeGraph::new();
        let a = make_node(1, "A", NodeKind::Class);
        let b = make_node(2, "B", NodeKind::Class);
        let c = make_node(3, "C", NodeKind::Class);
        graph.add_node(a.clone());
        graph.add_node(b.clone());
        graph.add_node(c.clone());

        graph.add_edge(make_edge(10, 3, 2, EdgeKind::Extends)); // C -> B
        graph.add_edge(make_edge(11, 2, 1, EdgeKind::Extends)); // B -> A

        let mut cache = MroCache::new();
        let mro = first_wins_mro(&graph, c.id, &mut cache);
        let ids: Vec<u64> = mro.iter().map(|n| n.0).collect();
        assert_eq!(ids, vec![3, 2, 1]);
    }
}
