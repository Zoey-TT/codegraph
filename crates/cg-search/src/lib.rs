//! CodeGraph — Search: full-text (Tantivy) and in-memory name matching.
//!
//! The minimal release does not include semantic vector search because the
//! embedding backends are not yet implemented.

use cg_common::{CodeNode, EdgeKind, NodeKind};
use cg_graph::{Direction, GraphStore};

pub mod hybrid;
pub use hybrid::HybridSearcher;

pub mod tantivy_index;
pub use tantivy_index::TantivyIndex;

// ============================================================================
// SearchHit
// ============================================================================

/// A single search result hit.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub node: CodeNode,
    pub score: f64,
    pub source: SearchSource,
}

/// Which subsystem produced this hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSource {
    Fts,
    Memory,
}

// ============================================================================
// MemorySearcher
// ============================================================================

/// In-memory searcher that scans the graph directly.
///
/// Suitable for small-to-medium repos and as a fallback when
/// Tantivy indexes are not available.
pub struct MemorySearcher<'a> {
    store: &'a dyn GraphStore,
}

impl<'a> MemorySearcher<'a> {
    /// Create a new searcher backed by the given graph store.
    pub fn new(store: &'a dyn GraphStore) -> Self {
        Self { store }
    }

    /// Search nodes by name (substring match).
    pub fn search_name(
        &self,
        query: &str,
        kind_filter: Option<NodeKind>,
    ) -> anyhow::Result<Vec<SearchHit>> {
        let query_lower = query.to_lowercase();

        let candidates = if let Some(filter) = kind_filter {
            self.store.query_by_label(filter)?
        } else {
            self.store.all_nodes()?
        };

        let mut hits = Vec::new();
        for node in candidates {
            let name_lower = node.properties.name.to_lowercase();
            let score = score_name_match(&name_lower, &query_lower);
            if score > 0.0 {
                hits.push(SearchHit {
                    node,
                    score,
                    source: SearchSource::Memory,
                });
            }
        }

        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        Ok(hits)
    }

    /// Get 360° context for a symbol: neighbors + process membership.
    pub fn context(&self, node_id: u64) -> anyhow::Result<SymbolContext> {
        let id_str = node_id.to_string();
        let node = self
            .store
            .get_node(&id_str)?
            .ok_or_else(|| anyhow::anyhow!("node not found"))?;

        let outgoing = self
            .store
            .query_neighbors(&id_str, Direction::Outgoing, None)?;
        let incoming = self
            .store
            .query_neighbors(&id_str, Direction::Incoming, None)?;

        let mut calls = Vec::new();
        let mut callers = Vec::new();
        let mut imports = Vec::new();
        let mut members = Vec::new();

        for edge in &outgoing {
            match edge.kind {
                EdgeKind::Calls => calls.push(edge.target_id.0),
                EdgeKind::Contains => members.push(edge.target_id.0),
                EdgeKind::Imports => imports.push(edge.target_id.0),
                _ => {}
            }
        }

        for edge in &incoming {
            match edge.kind {
                EdgeKind::Calls => callers.push(edge.source_id.0),
                EdgeKind::Contains => {} // parent handled separately
                _ => {}
            }
        }

        Ok(SymbolContext {
            node,
            callers,
            calls,
            members,
            imports,
        })
    }
}

/// 360° context view for a single symbol.
#[derive(Debug, Clone)]
pub struct SymbolContext {
    pub node: CodeNode,
    pub callers: Vec<u64>,
    pub calls: Vec<u64>,
    pub members: Vec<u64>,
    pub imports: Vec<u64>,
}

/// Score a name match: exact (1.0) > prefix (0.8) > contains (0.5).
fn score_name_match(name: &str, query: &str) -> f64 {
    if name == query {
        1.0
    } else if name.starts_with(query) {
        0.8
    } else if name.contains(query) {
        0.5
    } else {
        0.0
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_exact_match() {
        assert_eq!(score_name_match("main", "main"), 1.0);
    }

    #[test]
    fn score_prefix_match() {
        assert_eq!(score_name_match("main_async", "main"), 0.8);
    }

    #[test]
    fn score_contains_match() {
        assert_eq!(score_name_match("foo_main_bar", "main"), 0.5);
    }

    #[test]
    fn score_no_match() {
        assert_eq!(score_name_match("foo", "bar"), 0.0);
    }
}
