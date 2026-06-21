//! Community detection using the Leiden algorithm.
//!
//! Detects functional communities in the code graph by grouping tightly
//! connected symbols (functions, structs, etc.) that call and import each other.

use std::collections::{HashMap, HashSet};

use petgraph::graph::{NodeIndex, UnGraph};
use petgraph::visit::EdgeRef;

use cg_common::{CodeEdge, EdgeKind, NodeId};

use crate::KnowledgeGraph;

// ============================================================================
// Public API
// ============================================================================

/// A detected functional community.
#[derive(Debug, Clone)]
pub struct Community {
    /// Unique community identifier.
    pub id: usize,
    /// Members (original `NodeId`s).
    pub members: Vec<NodeId>,
    /// Heuristic label derived from member names.
    pub label: String,
    /// Cohesion score: internal edges / possible edges [0, 1].
    pub cohesion: f64,
}

/// Configuration for the community detector.
#[derive(Debug, Clone, Copy)]
pub struct CommunityDetector {
    /// Resolution parameter γ (higher = more communities). Default 1.0.
    pub resolution: f64,
    /// Max iterations per local-moving phase. Default 100.
    pub max_iterations: usize,
    /// Minimum community size to keep. Default 2.
    pub min_size: usize,
}

impl Default for CommunityDetector {
    fn default() -> Self {
        // γ=0.5 works better for sparse code graphs (real-world repos are
        // much sparser than the complete-graph test fixtures).
        Self {
            resolution: 0.5,
            max_iterations: 100,
            min_size: 2,
        }
    }
}

impl CommunityDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Detect communities in the knowledge graph.
    ///
    /// Returns a list of communities with members, labels, and cohesion scores.
    pub fn detect(&self, kg: &KnowledgeGraph) -> Vec<Community> {
        // Build a petgraph from the knowledge graph (CALLS + CONTAINS + IMPORTS)
        let (graph, id_to_index, index_to_id) = build_graph(kg);

        if graph.node_count() == 0 {
            return Vec::new();
        }

        // Run simplified Leiden: local-moving + refinement + aggregation loop
        let communities = leiden(&graph, self.resolution, self.max_iterations);

        // Build Community structs
        let mut community_members: HashMap<usize, Vec<NodeId>> = HashMap::new();
        for (i, &c) in communities.iter().enumerate() {
            community_members.entry(c).or_default().push(index_to_id[i]);
        }

        let mut result = Vec::new();
        for (id, members) in community_members {
            if members.len() < self.min_size {
                continue;
            }
            let cohesion = compute_cohesion(&graph, &communities, id, &id_to_index);
            let label = generate_label(&members, kg);
            result.push(Community {
                id,
                members,
                label,
                cohesion,
            });
        }

        // Re-number IDs sequentially
        result.sort_by_key(|b| std::cmp::Reverse(b.members.len()));
        for (i, c) in result.iter_mut().enumerate() {
            c.id = i;
        }

        result
    }
}

// ============================================================================
// Graph construction
// ============================================================================

fn build_graph(kg: &KnowledgeGraph) -> (UnGraph<NodeId, f64>, HashMap<NodeId, usize>, Vec<NodeId>) {
    let n = kg.node_count();
    let mut graph = UnGraph::<NodeId, f64>::new_undirected();
    let mut id_to_index: HashMap<NodeId, usize> = HashMap::with_capacity(n);
    let mut index_to_id: Vec<NodeId> = Vec::with_capacity(n);

    // Add nodes
    for entry in kg.nodes.iter() {
        let id = *entry.key();
        let idx = graph.add_node(id).index();
        id_to_index.insert(id, idx);
        index_to_id.push(id);
    }

    // Add edges (undirected, deduplicated)
    let mut edge_set: HashSet<(usize, usize)> = HashSet::new();
    for entry in kg.out_edges.iter() {
        let source_id = *entry.key();
        let Some(&source_idx) = id_to_index.get(&source_id) else {
            continue;
        };
        for edge in entry.value() {
            if !is_community_edge(edge) {
                continue;
            }
            let target_id = edge.target_id;
            let Some(&target_idx) = id_to_index.get(&target_id) else {
                continue;
            };
            if source_idx == target_idx {
                continue;
            }
            let key = if source_idx < target_idx {
                (source_idx, target_idx)
            } else {
                (target_idx, source_idx)
            };
            if edge_set.insert(key) {
                graph.add_edge(NodeIndex::new(source_idx), NodeIndex::new(target_idx), 1.0);
            }
        }
    }

    (graph, id_to_index, index_to_id)
}

/// Which edge kinds contribute to community structure.
fn is_community_edge(edge: &CodeEdge) -> bool {
    matches!(
        edge.kind,
        EdgeKind::Calls | EdgeKind::Contains | EdgeKind::Imports
    )
}

// ============================================================================
// Leiden algorithm (simplified: local-moving + refinement + aggregation)
// ============================================================================

/// Run the Leiden algorithm on a graph (recursive).
fn leiden(graph: &UnGraph<NodeId, f64>, resolution: f64, max_iterations: usize) -> Vec<usize> {
    let n = graph.node_count();
    if n == 0 {
        return Vec::new();
    }

    // 1. Local moving phase
    let initial: Vec<usize> = (0..n).collect();
    let mut communities = local_moving(graph, &initial, resolution, max_iterations);

    // 2. Refinement: split disconnected communities into connected components
    communities = refine(graph, &communities);

    // 3. Aggregate
    let (aggregated, node_to_agg) = aggregate(graph, &communities);

    if aggregated.node_count() == n {
        // No communities were merged — stop
        return communities;
    }

    // 4. Recursively run Leiden on aggregated graph
    let agg_communities = leiden(&aggregated, resolution, max_iterations);

    // 5. Map aggregated communities back to original nodes
    let mut result = vec![0; n];
    for i in 0..n {
        result[i] = agg_communities[node_to_agg[i]];
    }

    result
}

// ============================================================================
// Local moving phase
// ============================================================================

/// Fast local-moving optimisation (Louvain-style).
fn local_moving(
    graph: &UnGraph<NodeId, f64>,
    initial: &[usize],
    resolution: f64,
    max_iterations: usize,
) -> Vec<usize> {
    let n = graph.node_count();
    let mut communities = initial.to_vec();
    let mut sizes = vec![0usize; n.max(*initial.iter().max().unwrap_or(&0)) + 1];
    for &c in &communities {
        sizes[c] += 1;
    }

    let mut moved = true;
    let mut iteration = 0;

    while moved && iteration < max_iterations {
        moved = false;
        iteration += 1;

        for i in 0..n {
            let current_c = communities[i];
            let n_current = sizes[current_c];

            // Compute edge weight from node i to each neighboring community
            let mut comm_weights: HashMap<usize, f64> = HashMap::new();
            for edge in graph.edges(NodeIndex::new(i)) {
                let j = if edge.source().index() == i {
                    edge.target().index()
                } else {
                    edge.source().index()
                };
                let w = *edge.weight();
                let c = communities[j];
                *comm_weights.entry(c).or_insert(0.0) += w;
            }

            let k_i_current = comm_weights.get(&current_c).copied().unwrap_or(0.0);
            let mut best_gain = 0.0;
            let mut best_c = current_c;

            for (&c, &k_i_c) in &comm_weights {
                if c == current_c {
                    continue;
                }
                let n_c = sizes[c];
                // CPM gain formula (see module docs for derivation)
                let gain = k_i_c - k_i_current + resolution * (n_current as f64 - n_c as f64 - 1.0);
                if gain > best_gain {
                    best_gain = gain;
                    best_c = c;
                }
            }

            if best_c != current_c {
                communities[i] = best_c;
                sizes[current_c] -= 1;
                if best_c >= sizes.len() {
                    sizes.resize(best_c + 1, 0);
                }
                sizes[best_c] += 1;
                moved = true;
            }
        }
    }

    // Renumber communities sequentially (some may have become empty)
    let mut renumber: HashMap<usize, usize> = HashMap::new();
    let mut next_id = 0;
    for c in &mut communities {
        *c = *renumber.entry(*c).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
    }

    communities
}

// ============================================================================
// Refinement — guarantee each community is connected
// ============================================================================

/// Split each community into its connected components.
fn refine(graph: &UnGraph<NodeId, f64>, communities: &[usize]) -> Vec<usize> {
    let n = graph.node_count();
    let max_c = *communities.iter().max().unwrap_or(&0);
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); max_c + 1];
    for (i, &c) in communities.iter().enumerate() {
        members[c].push(i);
    }

    let mut new_communities = vec![0usize; n];
    let mut next_id = 0;

    for member_list in members {
        if member_list.len() <= 1 {
            for &i in &member_list {
                new_communities[i] = next_id;
            }
            if !member_list.is_empty() {
                next_id += 1;
            }
            continue;
        }

        let member_set: HashSet<usize> = member_list.iter().copied().collect();

        // For each unvisited node in member_list, BFS to find connected components
        let mut local_visited = HashSet::new();
        for &start in &member_list {
            if local_visited.contains(&start) {
                continue;
            }
            let mut component = Vec::new();
            let mut stack = vec![start];
            local_visited.insert(start);
            while let Some(u) = stack.pop() {
                component.push(u);
                for edge in graph.edges(NodeIndex::new(u)) {
                    let v = if edge.source().index() == u {
                        edge.target().index()
                    } else {
                        edge.source().index()
                    };
                    if member_set.contains(&v) && !local_visited.contains(&v) {
                        local_visited.insert(v);
                        stack.push(v);
                    }
                }
            }
            for &node in &component {
                new_communities[node] = next_id;
            }
            next_id += 1;
        }
    }

    new_communities
}

// ============================================================================
// Aggregation
// ============================================================================

/// Aggregate communities into a super-node graph.
fn aggregate(
    graph: &UnGraph<NodeId, f64>,
    communities: &[usize],
) -> (UnGraph<NodeId, f64>, Vec<usize>) {
    let _n = graph.node_count();
    let max_c = *communities.iter().max().unwrap_or(&0);
    let num_communities = max_c + 1;

    let mut aggregated = UnGraph::<NodeId, f64>::new_undirected();
    for c in 0..num_communities {
        // Use a dummy NodeId based on community index
        aggregated.add_node(NodeId::new(c as u64));
    }

    let mut edge_weights: HashMap<(usize, usize), f64> = HashMap::new();

    for edge in graph.edge_indices() {
        let (source, target) = graph.edge_endpoints(edge).unwrap();
        let s = source.index();
        let t = target.index();
        let c_s = communities[s];
        let c_t = communities[t];
        let w = graph[edge];

        if c_s == c_t {
            // Self-loop
            let key = (c_s, c_s);
            *edge_weights.entry(key).or_insert(0.0) += w;
        } else {
            let key = if c_s < c_t { (c_s, c_t) } else { (c_t, c_s) };
            *edge_weights.entry(key).or_insert(0.0) += w;
        }
    }

    for ((u, v), w) in edge_weights {
        aggregated.add_edge(NodeIndex::new(u), NodeIndex::new(v), w);
    }

    // node_to_agg: original node index -> aggregated node index
    let node_to_agg = communities.to_vec();

    (aggregated, node_to_agg)
}

// ============================================================================
// Helpers
// ============================================================================

/// Cohesion = internal edges / (n choose 2).
fn compute_cohesion(
    graph: &UnGraph<NodeId, f64>,
    communities: &[usize],
    community_id: usize,
    _id_map: &HashMap<NodeId, usize>,
) -> f64 {
    let members: Vec<usize> = communities
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == community_id)
        .map(|(i, _)| i)
        .collect();

    let n = members.len();
    if n <= 1 {
        return 1.0;
    }

    let member_set: HashSet<usize> = members.iter().copied().collect();
    let mut internal = 0.0;

    for &i in &members {
        for edge in graph.edges(NodeIndex::new(i)) {
            let j = if edge.source().index() == i {
                edge.target().index()
            } else {
                edge.source().index()
            };
            if member_set.contains(&j) {
                internal += *edge.weight();
            }
        }
    }

    // Each internal edge counted twice in undirected graph
    internal /= 2.0;

    let possible = (n * (n - 1) / 2) as f64;
    if possible == 0.0 {
        1.0
    } else {
        (internal / possible).min(1.0)
    }
}

/// Generate a heuristic label from community member names.
///
/// Tokenises names by underscore and camelCase, then picks the most frequent
/// token after filtering stop-words.
fn generate_label(members: &[NodeId], kg: &KnowledgeGraph) -> String {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "the", "and", "or", "of", "to", "in", "for", "on", "with", "get", "set", "new",
        "from", "into", "as", "is", "has", "can", "do", "fn", "func", "function", "test", "tests",
        "impl", "mod", "pub",
    ];

    let mut freq: HashMap<String, usize> = HashMap::new();

    for &id in members {
        if let Some(node) = kg.nodes.get(&id) {
            if !is_label_relevant_kind(node.kind) {
                continue;
            }
            for token in tokenise_name(&node.properties.name) {
                let lower = token.to_lowercase();
                if STOP_WORDS.contains(&lower.as_str()) {
                    continue;
                }
                if lower.len() < 2 {
                    continue;
                }
                *freq.entry(lower).or_insert(0) += 1;
            }
        }
    }

    if freq.is_empty() {
        return "unknown".to_string();
    }

    // Pick the token with highest frequency, breaking ties by alphabetical order
    // for determinism.
    let (best, _) = freq
        .into_iter()
        .max_by(|(a_word, a_count), (b_word, b_count)| {
            a_count.cmp(b_count).then_with(|| a_word.cmp(b_word))
        })
        .unwrap();

    best
}

fn is_label_relevant_kind(kind: cg_common::NodeKind) -> bool {
    matches!(
        kind,
        cg_common::NodeKind::Function
            | cg_common::NodeKind::Method
            | cg_common::NodeKind::Struct
            | cg_common::NodeKind::Class
            | cg_common::NodeKind::Trait
            | cg_common::NodeKind::Module
            | cg_common::NodeKind::Enum
            | cg_common::NodeKind::Interface
    )
}

/// Split a name into tokens by underscore and camelCase boundaries.
fn tokenise_name(name: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for underscore_part in name.split('_') {
        if underscore_part.is_empty() {
            continue;
        }
        // Split camelCase / PascalCase
        let mut current = String::new();
        for (i, ch) in underscore_part.chars().enumerate() {
            if i > 0 && ch.is_uppercase() {
                if !current.is_empty() {
                    tokens.push(current.clone());
                }
                current = ch.to_lowercase().to_string();
            } else {
                current.push(ch);
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
    }
    tokens
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cg_common::{CodeNode, NodeKind, NodeProperties};

    fn make_node(id: u64, name: &str) -> CodeNode {
        CodeNode::new(
            NodeId::new(id),
            NodeKind::Function,
            NodeProperties::new(name, "/test.rs"),
        )
    }

    fn make_edge(id: u64, from: u64, to: u64, kind: EdgeKind) -> CodeEdge {
        CodeEdge::new(
            NodeId::new(id),
            NodeId::new(from),
            NodeId::new(to),
            kind,
            1.0,
            "test",
        )
    }

    #[test]
    fn detect_two_cliques() {
        let kg = KnowledgeGraph::new();

        // Clique 1: nodes 1,2,3
        for i in 1..=3 {
            kg.add_node(make_node(i, &format!("fn_{}", i)));
        }
        // Clique 2: nodes 4,5,6
        for i in 4..=6 {
            kg.add_node(make_node(i, &format!("fn_{}", i)));
        }

        // Edges within clique 1
        kg.add_edge(make_edge(100, 1, 2, EdgeKind::Calls));
        kg.add_edge(make_edge(101, 2, 1, EdgeKind::Calls));
        kg.add_edge(make_edge(102, 2, 3, EdgeKind::Calls));
        kg.add_edge(make_edge(103, 3, 1, EdgeKind::Calls));

        // Edges within clique 2
        kg.add_edge(make_edge(104, 4, 5, EdgeKind::Calls));
        kg.add_edge(make_edge(105, 5, 4, EdgeKind::Calls));
        kg.add_edge(make_edge(106, 5, 6, EdgeKind::Calls));
        kg.add_edge(make_edge(107, 6, 4, EdgeKind::Calls));

        // One bridge edge between cliques
        kg.add_edge(make_edge(108, 3, 4, EdgeKind::Calls));

        // Use γ=0.5 so that the tightly connected cliques are merged internally.
        let detector = CommunityDetector {
            resolution: 0.5,
            ..Default::default()
        };
        let communities = detector.detect(&kg);

        assert!(!communities.is_empty());
        let total_members: usize = communities.iter().map(|c| c.members.len()).sum();
        assert_eq!(total_members, 6);
    }

    #[test]
    fn empty_graph_returns_empty() {
        let kg = KnowledgeGraph::new();
        let detector = CommunityDetector::new();
        let communities = detector.detect(&kg);
        assert!(communities.is_empty());
    }

    #[test]
    fn single_node() {
        let kg = KnowledgeGraph::new();
        kg.add_node(make_node(1, "main"));
        let detector = CommunityDetector::new();
        let communities = detector.detect(&kg);
        // min_size=2 filters out single-node communities
        assert!(communities.is_empty());
    }

    #[test]
    fn cohesion_perfect_clique() {
        let kg = KnowledgeGraph::new();
        for i in 1..=4 {
            kg.add_node(make_node(i, &format!("fn_{}", i)));
        }
        // Complete graph K4
        for i in 1..=4 {
            for j in (i + 1)..=4 {
                kg.add_edge(make_edge(i * 10 + j, i, j, EdgeKind::Calls));
            }
        }

        let detector = CommunityDetector {
            resolution: 0.5,
            min_size: 2,
            ..Default::default()
        };
        let communities = detector.detect(&kg);
        assert_eq!(communities.len(), 1);
        assert_eq!(communities[0].members.len(), 4);
        assert!((communities[0].cohesion - 1.0).abs() < 1e-9);
    }
}
