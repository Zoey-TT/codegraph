//! Full call resolution with three-tier lookup + receiver inference.

use cg_common::{CodeEdge, EdgeKind, NodeId, NodeKind};
use cg_core::KnowledgeGraph;
use cg_core::mro::{MroCache, find_method_in_mro};

use super::cross_file::CrossFileIndex;
use crate::extractors::{CallForm, ExtractedCall};

/// Result of resolving a single call site.
#[derive(Debug, Clone)]
pub struct ResolvedCall {
    pub target_id: NodeId,
    pub confidence: f64,
    pub reason: String,
}

/// Context needed for full resolution.
pub struct ResolveContext<'a> {
    pub graph: &'a KnowledgeGraph,
    pub cross_file: &'a CrossFileIndex,
    pub mro: &'a MroCache,
}

/// Resolve a call site using the full three-tier system + MRO dispatch.
///
/// Returns one or more candidate targets.  When multiple targets are
/// returned the caller should pick the highest-confidence edge (or
/// keep them all when overloads are possible).
pub fn resolve_call_full(
    ctx: &ResolveContext,
    caller_id: NodeId,
    call: &ExtractedCall,
) -> Vec<ResolvedCall> {
    let mut results = Vec::new();

    let caller = match ctx.graph.get_node(&caller_id) {
        Some(n) => n,
        None => return results,
    };
    let caller_file = &caller.properties.file_path;

    // ------------------------------------------------------------------
    // Tier 1: same-file lookup (highest confidence)
    // ------------------------------------------------------------------
    if let Some(targets) = ctx
        .cross_file
        .lookup_in_file(caller_file, &call.callee_name)
    {
        for &tid in targets {
            if tid != caller_id {
                results.push(ResolvedCall {
                    target_id: tid,
                    confidence: 0.95,
                    reason: "same-file".into(),
                });
            }
        }
    }

    // For free-function calls, if Tier 1 found something we can return early.
    if !results.is_empty() && call.call_form == CallForm::Free {
        return results;
    }

    // ------------------------------------------------------------------
    // Member / Constructor calls: try receiver type inference + MRO
    // ------------------------------------------------------------------
    if (call.call_form == CallForm::Member || call.call_form == CallForm::Constructor)
        && let Some(receiver_type) = infer_receiver_type(ctx, caller_file, &call.receiver_name)
        && let Some(method_id) =
            find_method_in_mro(ctx.graph, ctx.mro, receiver_type, &call.callee_name)
    {
        results.push(ResolvedCall {
            target_id: method_id,
            confidence: 0.92,
            reason: "mro-dispatch".into(),
        });
    }

    // ------------------------------------------------------------------
    // Tier 2: imported / cross-file scope
    // ------------------------------------------------------------------
    if (results.is_empty() || call.call_form == CallForm::Free)
        && let Some(global_targets) = ctx.cross_file.lookup_global(&call.callee_name)
    {
        for &tid in global_targets {
            if let Some(target) = ctx.graph.get_node(&tid)
                && target.properties.file_path != *caller_file
            {
                // Check whether there is an IMPORTS edge from the caller's file
                // to the target's file.  We approximate this by looking at
                // outgoing IMPORTS edges from any node in the caller file.
                let is_imported =
                    has_import_from_file(ctx.graph, caller_file, &target.properties.file_path);
                let confidence = if is_imported { 0.90 } else { 0.85 };
                let reason = if is_imported {
                    "imported-symbol"
                } else {
                    "cross-file"
                };
                results.push(ResolvedCall {
                    target_id: tid,
                    confidence,
                    reason: reason.into(),
                });
            }
        }
    }

    // ------------------------------------------------------------------
    // Tier 3: global fallback (lowest confidence)
    // ------------------------------------------------------------------
    if results.is_empty()
        && let Some(global_targets) = ctx.cross_file.lookup_global(&call.callee_name)
    {
        for &tid in global_targets {
            results.push(ResolvedCall {
                target_id: tid,
                confidence: 0.50,
                reason: "global-fallback".into(),
            });
        }
    }

    // Deduplicate by target_id, keeping the highest confidence
    dedup_by_target(results)
}

/// Infer the type (as a class-like NodeId) of a receiver expression.
///
/// Current heuristics:
/// 1. Look up the receiver name in the same file; if it is a Variable,
///    read its `extras["inferred_type"]`.
/// 2. If the receiver name matches a class-like node in the same file,
///    treat that as the type.
fn infer_receiver_type(
    ctx: &ResolveContext,
    caller_file: &std::path::PathBuf,
    receiver_name: &Option<String>,
) -> Option<NodeId> {
    let name = receiver_name.as_ref()?;

    // Heuristic 1: same-file variable with inferred_type
    if let Some(targets) = ctx.cross_file.lookup_in_file(caller_file, name) {
        for &tid in targets {
            if let Some(node) = ctx.graph.get_node(&tid) {
                if node.kind == NodeKind::Variable
                    && let Some(serde_json::Value::String(ty_name)) =
                        node.properties.extras.get("inferred_type")
                {
                    // Try to resolve the type name to a node
                    if let Some(type_targets) = ctx.cross_file.lookup_global(ty_name) {
                        for &type_id in type_targets {
                            if let Some(ty_node) = ctx.graph.get_node(&type_id)
                                && matches!(
                                    ty_node.kind,
                                    NodeKind::Class
                                        | NodeKind::Struct
                                        | NodeKind::Trait
                                        | NodeKind::Interface
                                )
                            {
                                return Some(type_id);
                            }
                        }
                    }
                }

                // Heuristic 2: receiver is itself a class/struct instance
                if matches!(
                    node.kind,
                    NodeKind::Class | NodeKind::Struct | NodeKind::Trait | NodeKind::Interface
                ) {
                    return Some(tid);
                }
            }
        }
    }

    // Heuristic 3: global type name match
    if let Some(targets) = ctx.cross_file.lookup_global(name) {
        for &tid in targets {
            if let Some(node) = ctx.graph.get_node(&tid)
                && matches!(
                    node.kind,
                    NodeKind::Class | NodeKind::Struct | NodeKind::Trait | NodeKind::Interface
                )
            {
                return Some(tid);
            }
        }
    }

    None
}

/// Check whether any node in `from_file` has an IMPORTS edge pointing
/// to a node whose file path is `to_file`.
fn has_import_from_file(
    graph: &KnowledgeGraph,
    from_file: &std::path::PathBuf,
    to_file: &std::path::PathBuf,
) -> bool {
    let from_nodes: Vec<NodeId> = graph
        .file_index
        .get(from_file)
        .map(|r| r.clone())
        .unwrap_or_default();

    for from_id in from_nodes {
        for edge in graph.outgoing_edges_by_kind(&from_id, EdgeKind::Imports) {
            if let Some(target) = graph.get_node(&edge.target_id)
                && target.properties.file_path == *to_file
            {
                return true;
            }
        }
    }

    false
}

/// Deduplicate results, keeping the highest confidence for each target.
fn dedup_by_target(results: Vec<ResolvedCall>) -> Vec<ResolvedCall> {
    use std::collections::HashMap;
    let mut best: HashMap<NodeId, ResolvedCall> = HashMap::new();
    for r in results {
        best.entry(r.target_id)
            .and_modify(|e| {
                if r.confidence > e.confidence {
                    *e = r.clone();
                }
            })
            .or_insert(r);
    }
    best.into_values().collect()
}

/// Convert a `ResolvedCall` into a `CodeEdge`.
pub fn resolved_call_to_edge(caller_id: NodeId, resolved: &ResolvedCall) -> CodeEdge {
    use rustc_hash::FxHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    caller_id.hash(&mut hasher);
    resolved.target_id.hash(&mut hasher);
    resolved.reason.hash(&mut hasher);
    let edge_id = NodeId::new(hasher.finish());

    CodeEdge::new(
        edge_id,
        caller_id,
        resolved.target_id,
        EdgeKind::Calls,
        resolved.confidence,
        &resolved.reason,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractors::{CallForm, ExtractedCall};
    use cg_common::{CodeNode, NodeKind, NodeProperties};
    use cg_core::KnowledgeGraph;
    use tree_sitter::Range;

    fn make_node(id: u64, name: &str, kind: NodeKind, file: &str) -> CodeNode {
        CodeNode::new(
            cg_common::NodeId::new(id),
            kind,
            NodeProperties::new(name, file),
        )
    }

    fn dummy_range() -> Range {
        Range {
            start_byte: 0,
            end_byte: 1,
            start_point: tree_sitter::Point { row: 0, column: 0 },
            end_point: tree_sitter::Point { row: 0, column: 1 },
        }
    }

    #[test]
    fn tier1_same_file() {
        let graph = KnowledgeGraph::new();
        let f = make_node(1, "foo", NodeKind::Function, "/a.rs");
        let b = make_node(2, "bar", NodeKind::Function, "/a.rs");
        graph.add_node(f.clone());
        graph.add_node(b.clone());

        let cross = CrossFileIndex::build(&graph);
        let mro = MroCache::new();
        let ctx = ResolveContext {
            graph: &graph,
            cross_file: &cross,
            mro: &mro,
        };

        let call = ExtractedCall {
            caller_id: f.id,
            callee_name: "bar".into(),
            call_form: CallForm::Free,
            range: dummy_range(),
            receiver_name: None,
            argument_count: 0,
        };

        let results = resolve_call_full(&ctx, f.id, &call);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_id, b.id);
        assert_eq!(results[0].confidence, 0.95);
    }

    #[test]
    fn tier3_global_fallback() {
        let graph = KnowledgeGraph::new();
        let f = make_node(1, "foo", NodeKind::Function, "/a.rs");
        let b = make_node(2, "bar", NodeKind::Function, "/b.rs");
        graph.add_node(f.clone());
        graph.add_node(b.clone());

        let cross = CrossFileIndex::build(&graph);
        let mro = MroCache::new();
        let ctx = ResolveContext {
            graph: &graph,
            cross_file: &cross,
            mro: &mro,
        };

        let call = ExtractedCall {
            caller_id: f.id,
            callee_name: "bar".into(),
            call_form: CallForm::Free,
            range: dummy_range(),
            receiver_name: None,
            argument_count: 0,
        };

        let results = resolve_call_full(&ctx, f.id, &call);
        // Tier 2 (cross-file) should find it before Tier 3
        assert!(
            results
                .iter()
                .any(|r| r.target_id == b.id && r.confidence >= 0.85),
            "Expected cross-file or global resolution for bar"
        );
    }
}
