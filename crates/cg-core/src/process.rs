//! Process (execution-flow) detection.
//!
//! Scores entry points, traces call chains, and classifies processes.

use std::collections::{HashSet, VecDeque};

use cg_common::{EdgeKind, NodeId, NodeKind};

use crate::KnowledgeGraph;

/// Classification of a detected process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessClass {
    /// All steps fall within a single community.
    IntraCommunity,
    /// Steps span multiple communities.
    CrossCommunity,
}

/// A single step in a traced process.
#[derive(Debug, Clone)]
pub struct ProcessStep {
    pub node_id: NodeId,
    pub depth: usize,
}

/// A detected execution flow.
#[derive(Debug, Clone)]
pub struct DetectedProcess {
    pub entry_id: NodeId,
    pub steps: Vec<ProcessStep>,
    pub classification: ProcessClass,
    pub entry_score: f64,
    pub entry_reason: String,
}

/// Result of process detection.
#[derive(Debug, Clone)]
pub struct ProcessDetectionResult {
    pub processes: Vec<DetectedProcess>,
}

/// Score potential entry points in the graph.
///
/// Returns a list of (node_id, score, reason) sorted by descending score.
pub fn score_entry_points(graph: &KnowledgeGraph) -> Vec<(NodeId, f64, String)> {
    let mut scores: Vec<(NodeId, f64, String)> = Vec::new();

    let candidate_kinds = [NodeKind::Function, NodeKind::Method, NodeKind::Route];

    for kind in &candidate_kinds {
        for node in graph.nodes_by_kind(*kind) {
            let mut score = 0.0;
            let mut reasons = Vec::new();

            let name = &node.properties.name;

            // 1. main / __main__
            if name == "main" || name == "__main__" {
                score += 1.0;
                reasons.push("main entry point");
            }

            // 2. Route handlers
            if *kind == NodeKind::Route {
                score += 0.9;
                reasons.push("HTTP route handler");
            } else if !graph
                .outgoing_edges_by_kind(&node.id, EdgeKind::HandlesRoute)
                .is_empty()
            {
                score += 0.9;
                reasons.push("route handler");
            }

            // 3. Framework entry points (name patterns)
            if is_framework_entry(name) {
                score += 0.8;
                reasons.push("framework entry pattern");
            }

            // 4. Public functions called from other files
            let incoming_calls = graph
                .incoming_sources(&node.id)
                .into_iter()
                .filter(|&src| {
                    graph
                        .get_node(&src)
                        .map(|n| n.properties.file_path != node.properties.file_path)
                        .unwrap_or(false)
                })
                .count();
            if incoming_calls > 0 {
                score += 0.7;
                reasons.push("called from other files");
            }

            // 5. Exported symbols (if is_exported flag is set)
            if node.properties.is_exported == Some(true) {
                score += 0.3;
                reasons.push("exported");
            }

            if score > 0.0 {
                scores.push((node.id, score, reasons.join(", ")));
            }
        }
    }

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scores
}

fn is_framework_entry(name: &str) -> bool {
    let patterns = [
        "getServerSideProps",
        "getStaticProps",
        "handler",
        "onRequest",
        "middleware",
        "init",
        "setup",
        "bootstrap",
        "start",
        "run",
        "execute",
        "handle",
        "process_request",
        "process_event",
    ];
    patterns.contains(&name)
}

/// Trace a call chain starting from an entry point via BFS.
///
/// Returns a list of steps including the entry point at depth 0.
/// Stops at `max_depth` or when a cycle is detected.
pub fn trace_call_chain(
    graph: &KnowledgeGraph,
    entry: NodeId,
    max_depth: usize,
) -> Vec<ProcessStep> {
    let mut steps = vec![ProcessStep {
        node_id: entry,
        depth: 0,
    }];
    let mut visited = HashSet::new();
    visited.insert(entry);

    let mut queue = VecDeque::new();
    queue.push_back((entry, 0usize));

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        for edge in graph.outgoing_edges_by_kind(&current, EdgeKind::Calls) {
            let target = edge.target_id;
            if target == current {
                continue; // skip self-recursion
            }
            if visited.insert(target) {
                steps.push(ProcessStep {
                    node_id: target,
                    depth: depth + 1,
                });
                queue.push_back((target, depth + 1));
            }
        }
    }

    steps
}

/// Classify a process as IntraCommunity or CrossCommunity.
pub fn classify_process(graph: &KnowledgeGraph, steps: &[ProcessStep]) -> ProcessClass {
    let mut community_ids = HashSet::new();

    for step in steps {
        for edge in graph.outgoing_edges_by_kind(&step.node_id, EdgeKind::MemberOf) {
            community_ids.insert(edge.target_id);
        }
    }

    if community_ids.len() <= 1 {
        ProcessClass::IntraCommunity
    } else {
        ProcessClass::CrossCommunity
    }
}

/// Detect all processes in the graph.
///
/// Takes the top-N entry points (score > threshold) and traces each.
pub fn detect_processes(graph: &KnowledgeGraph) -> ProcessDetectionResult {
    let entry_points = score_entry_points(graph);
    let threshold = 0.5;
    let max_depth = 10;
    let max_processes = 20;

    let mut processes = Vec::new();

    for (entry_id, score, reason) in entry_points {
        if score < threshold {
            break;
        }
        if processes.len() >= max_processes {
            break;
        }

        let steps = trace_call_chain(graph, entry_id, max_depth);
        if steps.len() <= 1 {
            continue; // Skip isolated entry points with no callees
        }

        let classification = classify_process(graph, &steps);

        processes.push(DetectedProcess {
            entry_id,
            steps,
            classification,
            entry_score: score,
            entry_reason: reason,
        });
    }

    ProcessDetectionResult { processes }
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
    fn score_main_function() {
        let graph = KnowledgeGraph::new();
        let main = make_node(1, "main", NodeKind::Function);
        let other = make_node(2, "helper", NodeKind::Function);
        graph.add_node(main.clone());
        graph.add_node(other.clone());

        let scores = score_entry_points(&graph);
        assert_eq!(scores.len(), 1);
        assert_eq!(scores[0].0, main.id);
        assert!(scores[0].1 >= 1.0);
    }

    #[test]
    fn trace_call_chain_bfs() {
        let graph = KnowledgeGraph::new();
        let a = make_node(1, "a", NodeKind::Function);
        let b = make_node(2, "b", NodeKind::Function);
        let c = make_node(3, "c", NodeKind::Function);
        graph.add_node(a.clone());
        graph.add_node(b.clone());
        graph.add_node(c.clone());

        graph.add_edge(make_edge(10, 1, 2, EdgeKind::Calls));
        graph.add_edge(make_edge(11, 2, 3, EdgeKind::Calls));

        let steps = trace_call_chain(&graph, a.id, 5);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].depth, 0);
        assert_eq!(steps[1].depth, 1);
        assert_eq!(steps[2].depth, 2);
    }

    #[test]
    fn classify_cross_community() {
        let graph = KnowledgeGraph::new();
        let a = make_node(1, "a", NodeKind::Function);
        let c1 = make_node(10, "comm1", NodeKind::Community);
        let c2 = make_node(11, "comm2", NodeKind::Community);
        graph.add_node(a.clone());
        graph.add_node(c1.clone());
        graph.add_node(c2.clone());

        graph.add_edge(make_edge(20, 1, 10, EdgeKind::MemberOf));
        graph.add_edge(make_edge(21, 1, 11, EdgeKind::MemberOf));

        let steps = vec![ProcessStep {
            node_id: a.id,
            depth: 0,
        }];
        let class = classify_process(&graph, &steps);
        assert_eq!(class, ProcessClass::CrossCommunity);
    }
}
