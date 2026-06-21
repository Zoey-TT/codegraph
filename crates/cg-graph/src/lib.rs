//! CodeGraph — Graph storage adapter and schema management.
//!
//! Provides an in-memory backend:
//! - `InMemoryGraphStore`: wraps `cg_core::KnowledgeGraph` (DashMap)

use std::io::Write;
use std::sync::Arc;

use cg_common::{CodeEdge, CodeNode, EdgeKind, NodeKind};
use cg_core::{ApplyDelta, GraphDelta, KnowledgeGraph};

// ============================================================================
// Direction
// ============================================================================

/// Direction for neighbor queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Outgoing,
    Incoming,
    Both,
}

// ============================================================================
// QueryResult
// ============================================================================

/// Result of a Cypher query.
#[derive(Debug, Clone, Default)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

// ============================================================================
// GraphStore trait
// ============================================================================

/// Unified graph storage interface.
///
/// `InMemoryGraphStore` implements this trait.
pub trait GraphStore: Send + Sync {
    /// Add a single node.
    fn add_node(&self, node: CodeNode) -> anyhow::Result<()>;

    /// Add a single edge.
    fn add_edge(&self, edge: CodeEdge) -> anyhow::Result<()>;

    /// Get a node by string id.
    fn get_node(&self, id: &str) -> anyhow::Result<Option<CodeNode>>;

    /// Query neighbors of a node.
    fn query_neighbors(
        &self,
        id: &str,
        direction: Direction,
        edge_kind: Option<EdgeKind>,
    ) -> anyhow::Result<Vec<CodeEdge>>;

    /// Query nodes by label/kind.
    fn query_by_label(&self, label: NodeKind) -> anyhow::Result<Vec<CodeNode>>;

    /// Query all nodes (regardless of kind).
    fn all_nodes(&self) -> anyhow::Result<Vec<CodeNode>>;

    /// Execute a raw Cypher query.
    fn query_cypher(&self, query: &str) -> anyhow::Result<QueryResult>;

    /// Create a full-text search index.
    fn create_fts_index(&self, table: &str, fields: &[&str]) -> anyhow::Result<()>;

    /// Create a vector (HNSW) index.
    fn create_vector_index(
        &self,
        table: &str,
        field: &str,
        dims: usize,
        metric: &str,
    ) -> anyhow::Result<()>;

    /// Batch insert nodes (preferred for pipeline bulk load).
    fn batch_insert_nodes(&self, nodes: Vec<CodeNode>) -> anyhow::Result<()>;

    /// Batch insert edges (preferred for pipeline bulk load).
    fn batch_insert_edges(&self, edges: Vec<CodeEdge>) -> anyhow::Result<()>;
}

// ============================================================================
// InMemoryGraphStore
// ============================================================================

/// In-memory graph store wrapping `cg_core::KnowledgeGraph`.
///
/// Suitable for pipeline execution and fast queries during indexing.
#[derive(Debug, Clone, Default)]
pub struct InMemoryGraphStore {
    inner: Arc<KnowledgeGraph>,
}

impl InMemoryGraphStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(KnowledgeGraph::new()),
        }
    }

    pub fn from_knowledge_graph(graph: KnowledgeGraph) -> Self {
        Self {
            inner: Arc::new(graph),
        }
    }

    /// Access the underlying `KnowledgeGraph`.
    pub fn knowledge_graph(&self) -> &KnowledgeGraph {
        &self.inner
    }

    /// Convert into the underlying `KnowledgeGraph`.
    pub fn into_knowledge_graph(self) -> Option<KnowledgeGraph> {
        Arc::try_unwrap(self.inner).ok()
    }

    /// Export all nodes and edges to JSONL files in the given directory.
    pub fn export_jsonl(&self, dir: &std::path::Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;
        let nodes_path = dir.join("nodes.jsonl");
        let edges_path = dir.join("edges.jsonl");
        let mut nodes_file = std::io::BufWriter::new(std::fs::File::create(nodes_path)?);
        let mut edges_file = std::io::BufWriter::new(std::fs::File::create(edges_path)?);

        for entry in self.inner.nodes.iter() {
            serde_json::to_writer(&mut nodes_file, entry.value())?;
            nodes_file.write_all(b"\n")?;
        }
        for entry in self.inner.out_edges.iter() {
            for edge in entry.value() {
                serde_json::to_writer(&mut edges_file, edge)?;
                edges_file.write_all(b"\n")?;
            }
        }
        Ok(())
    }

    /// Export all nodes and edges to CSV files for external bulk import.
    pub fn export_csv(
        &self,
        dir: &std::path::Path,
    ) -> anyhow::Result<(std::path::PathBuf, std::path::PathBuf)> {
        std::fs::create_dir_all(dir)?;
        let nodes_path = dir.join("nodes.csv");
        let edges_path = dir.join("edges.csv");
        let mut nodes_file = std::io::BufWriter::new(std::fs::File::create(&nodes_path)?);
        let mut edges_file = std::io::BufWriter::new(std::fs::File::create(&edges_path)?);

        // CSV header for nodes
        writeln!(
            nodes_file,
            "id,kind,name,file_path,language,start_line,end_line,extra_json"
        )?;

        for entry in self.inner.nodes.iter() {
            let node = entry.value();
            let lang = node
                .properties
                .language
                .map(|l| format!("{:?}", l))
                .unwrap_or_default();
            let extra = serde_json::to_string(&node.properties.extras)?;
            writeln!(
                nodes_file,
                "{},{},{},{},{},{},{},\"{}\"",
                node.id.0,
                csv_escape(&format!("{:?}", node.kind)),
                csv_escape(&node.properties.name),
                csv_escape(&node.properties.file_path.to_string_lossy()),
                csv_escape(&lang),
                node.properties.start_line.unwrap_or(0),
                node.properties.end_line.unwrap_or(0),
                csv_escape(&extra),
            )?;
        }

        // CSV header for edges
        writeln!(edges_file, "from_id,to_id,kind,confidence,reason,step")?;

        for entry in self.inner.out_edges.iter() {
            for edge in entry.value() {
                writeln!(
                    edges_file,
                    "{},{},{},{},{},{}",
                    edge.source_id.0,
                    edge.target_id.0,
                    csv_escape(&format!("{:?}", edge.kind)),
                    edge.confidence,
                    csv_escape(&edge.reason),
                    edge.step.unwrap_or(0),
                )?;
            }
        }

        Ok((nodes_path, edges_path))
    }

    /// Import nodes and edges from JSONL files in the given directory.
    pub fn import_jsonl(dir: &std::path::Path) -> anyhow::Result<Self> {
        let store = Self::new();
        let nodes_path = dir.join("nodes.jsonl");
        let edges_path = dir.join("edges.jsonl");

        if nodes_path.exists() {
            let file = std::io::BufReader::new(std::fs::File::open(nodes_path)?);
            for line in std::io::BufRead::lines(file) {
                let node: CodeNode = serde_json::from_str(&line?)?;
                store.add_node(node)?;
            }
        }
        if edges_path.exists() {
            let file = std::io::BufReader::new(std::fs::File::open(edges_path)?);
            for line in std::io::BufRead::lines(file) {
                let edge: CodeEdge = serde_json::from_str(&line?)?;
                store.add_edge(edge)?;
            }
        }
        Ok(store)
    }
}

impl GraphStore for InMemoryGraphStore {
    fn add_node(&self, node: CodeNode) -> anyhow::Result<()> {
        self.inner.apply_delta(GraphDelta {
            nodes_to_add: vec![node],
            ..Default::default()
        });
        Ok(())
    }

    fn add_edge(&self, edge: CodeEdge) -> anyhow::Result<()> {
        self.inner.apply_delta(GraphDelta {
            edges_to_add: vec![edge],
            ..Default::default()
        });
        Ok(())
    }

    fn get_node(&self, id: &str) -> anyhow::Result<Option<CodeNode>> {
        let node_id = parse_node_id(id)?;
        Ok(self.inner.nodes.get(&node_id).map(|n| n.clone()))
    }

    fn query_neighbors(
        &self,
        id: &str,
        direction: Direction,
        edge_kind: Option<EdgeKind>,
    ) -> anyhow::Result<Vec<CodeEdge>> {
        let node_id = parse_node_id(id)?;
        let mut result = Vec::new();

        match direction {
            Direction::Outgoing | Direction::Both => {
                if let Some(edges) = self.inner.out_edges.get(&node_id) {
                    for edge in edges.iter() {
                        if edge_kind.is_none_or(|k| edge.kind == k) {
                            result.push(edge.clone());
                        }
                    }
                }
            }
            _ => {}
        }

        match direction {
            Direction::Incoming | Direction::Both => {
                if let Some(source_ids) = self.inner.in_edges.get(&node_id) {
                    for source_id in source_ids.iter() {
                        if let Some(out_edges) = self.inner.out_edges.get(source_id) {
                            for edge in out_edges.iter() {
                                if edge.target_id == node_id
                                    && edge_kind.is_none_or(|k| edge.kind == k)
                                {
                                    result.push(edge.clone());
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        Ok(result)
    }

    fn query_by_label(&self, label: NodeKind) -> anyhow::Result<Vec<CodeNode>> {
        let mut result = Vec::new();
        for entry in self.inner.nodes.iter() {
            if entry.value().kind == label {
                result.push(entry.value().clone());
            }
        }
        Ok(result)
    }

    fn all_nodes(&self) -> anyhow::Result<Vec<CodeNode>> {
        let mut result = Vec::new();
        for entry in self.inner.nodes.iter() {
            result.push(entry.value().clone());
        }
        Ok(result)
    }

    fn query_cypher(&self, _query: &str) -> anyhow::Result<QueryResult> {
        anyhow::bail!("Cypher queries are not supported in this build")
    }

    fn create_fts_index(&self, _table: &str, _fields: &[&str]) -> anyhow::Result<()> {
        // No-op for in-memory store
        Ok(())
    }

    fn create_vector_index(
        &self,
        _table: &str,
        _field: &str,
        _dims: usize,
        _metric: &str,
    ) -> anyhow::Result<()> {
        // No-op for in-memory store
        Ok(())
    }

    fn batch_insert_nodes(&self, nodes: Vec<CodeNode>) -> anyhow::Result<()> {
        self.inner.apply_delta(GraphDelta {
            nodes_to_add: nodes,
            ..Default::default()
        });
        Ok(())
    }

    fn batch_insert_edges(&self, edges: Vec<CodeEdge>) -> anyhow::Result<()> {
        self.inner.apply_delta(GraphDelta {
            edges_to_add: edges,
            ..Default::default()
        });
        Ok(())
    }
}

fn parse_node_id(id: &str) -> anyhow::Result<cg_common::NodeId> {
    let num = id.parse::<u64>()?;
    Ok(cg_common::NodeId::new(num))
}

/// Escape a string for CSV output.
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        s.to_string()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cg_common::{NodeId, NodeKind, NodeProperties};

    fn make_node(id: u64, kind: NodeKind, name: &str) -> CodeNode {
        CodeNode::new(NodeId::new(id), kind, NodeProperties::new(name, "test.rs"))
    }

    #[test]
    fn in_memory_add_and_get_node() {
        let store = InMemoryGraphStore::new();
        let node = make_node(1, NodeKind::Function, "main");
        store.add_node(node.clone()).unwrap();

        let fetched = store.get_node("1").unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().id, NodeId::new(1));
    }

    #[test]
    fn in_memory_query_by_label() {
        let store = InMemoryGraphStore::new();
        store
            .add_node(make_node(1, NodeKind::Function, "foo"))
            .unwrap();
        store
            .add_node(make_node(2, NodeKind::Struct, "Bar"))
            .unwrap();
        store
            .add_node(make_node(3, NodeKind::Function, "baz"))
            .unwrap();

        let functions = store.query_by_label(NodeKind::Function).unwrap();
        assert_eq!(functions.len(), 2);
    }

    #[test]
    fn in_memory_batch_insert() {
        let store = InMemoryGraphStore::new();
        let nodes = vec![
            make_node(1, NodeKind::Function, "a"),
            make_node(2, NodeKind::Function, "b"),
        ];
        store.batch_insert_nodes(nodes).unwrap();
        assert_eq!(store.get_node("1").unwrap().unwrap().properties.name, "a");
        assert_eq!(store.get_node("2").unwrap().unwrap().properties.name, "b");
    }

    #[test]
    fn export_csv_roundtrip() {
        let store = InMemoryGraphStore::new();
        store
            .add_node(make_node(1, NodeKind::Function, "main"))
            .unwrap();
        store
            .add_node(make_node(2, NodeKind::Struct, "User"))
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let (nodes_csv, edges_csv) = store.export_csv(tmp.path()).unwrap();

        assert!(nodes_csv.exists());
        assert!(edges_csv.exists());

        let nodes_content = std::fs::read_to_string(&nodes_csv).unwrap();
        assert!(nodes_content.contains("id,kind,name,file_path"));
        assert!(nodes_content.contains("main"));
        assert!(nodes_content.contains("User"));
    }
}
