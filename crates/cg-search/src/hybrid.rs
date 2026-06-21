//! HybridSearcher — unifies BM25 (Tantivy) and in-memory substring search.
//!
//! Semantic vector search has been removed from the minimal release because
//! the embedding backends (ONNX Runtime / Candle / Remote) are not yet
//! implemented. This searcher falls back to memory substring matching when
//! Tantivy is unavailable.

use cg_graph::GraphStore;

use crate::{MemorySearcher, SearchHit, TantivyIndex};

// ============================================================================
// HybridSearcher
// ============================================================================

/// Unified search interface combining BM25 and in-memory substring search.
pub struct HybridSearcher<'a> {
    store: &'a dyn GraphStore,
    tantivy: Option<TantivyIndex>,
}

impl<'a> HybridSearcher<'a> {
    /// Create a new searcher backed by the given graph store.
    pub fn new(store: &'a dyn GraphStore) -> Self {
        Self {
            store,
            tantivy: None,
        }
    }

    /// Enable Tantivy BM25 search with an existing index.
    pub fn with_tantivy(mut self, index: TantivyIndex) -> Self {
        self.tantivy = Some(index);
        self
    }

    /// Build an in-memory Tantivy index from the current graph store.
    pub fn build_tantivy(mut self) -> anyhow::Result<Self> {
        let mut index = TantivyIndex::create_in_memory()?;
        let nodes = self.store.all_nodes()?;
        index.index_nodes(&nodes)?;
        self.tantivy = Some(index);
        Ok(self)
    }

    /// Execute a hybrid search.
    ///
    /// - If Tantivy is configured, BM25 results are used.
    /// - Otherwise falls back to `MemorySearcher` substring matching.
    pub fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<SearchHit>> {
        let hits = if let Some(ref tantivy) = self.tantivy {
            tantivy.search(query, limit)?
        } else {
            let memory = MemorySearcher::new(self.store);
            memory.search_name(query, None)?
        };

        // Enrich hits with full nodes from the store (Tantivy nodes may be partial)
        let enriched: Vec<SearchHit> = hits
            .into_iter()
            .filter_map(|hit| {
                let id_str = hit.node.id.to_string();
                self.store
                    .get_node(&id_str)
                    .ok()
                    .flatten()
                    .map(|full_node| SearchHit {
                        node: full_node,
                        score: hit.score,
                        source: hit.source,
                    })
            })
            .collect();

        Ok(enriched)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cg_common::{CodeNode, NodeId, NodeKind, NodeProperties};
    use cg_graph::InMemoryGraphStore;

    fn make_node(id: u64, kind: NodeKind, name: &str, path: &str) -> CodeNode {
        CodeNode::new(NodeId::new(id), kind, NodeProperties::new(name, path))
    }

    fn populate_store() -> InMemoryGraphStore {
        let store = InMemoryGraphStore::new();
        store
            .add_node(make_node(1, NodeKind::Function, "main", "/src/main.rs"))
            .unwrap();
        store
            .add_node(make_node(
                2,
                NodeKind::Struct,
                "UserConfig",
                "/src/config.rs",
            ))
            .unwrap();
        store
            .add_node(make_node(
                3,
                NodeKind::Function,
                "parse_config",
                "/src/config.rs",
            ))
            .unwrap();
        store
    }

    #[test]
    fn hybrid_search_uses_tantivy_when_available() {
        let store = populate_store();
        let mut idx = TantivyIndex::create_in_memory().unwrap();
        idx.index_nodes(&store.all_nodes().unwrap()).unwrap();

        let searcher = HybridSearcher::new(&store).with_tantivy(idx);
        let hits = searcher.search("config", 10).unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn hybrid_search_fallback_to_memory() {
        let store = populate_store();
        let searcher = HybridSearcher::new(&store);
        let hits = searcher.search("config", 10).unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn hybrid_build_tantivy_auto() {
        let store = populate_store();
        let searcher = HybridSearcher::new(&store).build_tantivy().unwrap();
        let hits = searcher.search("main", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node.properties.name, "main");
    }
}
