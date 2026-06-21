//! Tantivy-based full-text search index for CodeGraph.
//!
//! Provides BM25 ranking over node names, file paths, and kinds
//! for small-to-medium repositories.

use std::path::Path;

use cg_common::{CodeNode, NodeKind};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{STORED, Schema, TEXT, Value};
use tantivy::{Index, IndexWriter, ReloadPolicy, Searcher, TantivyDocument};

use crate::{SearchHit, SearchSource};

// ============================================================================
// TantivyIndex
// ============================================================================

/// BM25 search index backed by Tantivy.
pub struct TantivyIndex {
    index: Index,
    schema: Schema,
    writer: Option<IndexWriter>,
}

impl TantivyIndex {
    // ------------------------------------------------------------------------
    // Construction
    // ------------------------------------------------------------------------

    /// Open (or create) a Tantivy index at the given directory path.
    pub fn open_or_create(dir: &Path) -> anyhow::Result<Self> {
        let schema = build_schema();
        let index = if dir.exists() {
            Index::open_in_dir(dir)?
        } else {
            std::fs::create_dir_all(dir)?;
            Index::create_in_dir(dir, schema.clone())?
        };
        Ok(Self {
            index,
            schema,
            writer: None,
        })
    }

    /// Create a purely in-memory index (useful for tests and small graphs).
    pub fn create_in_memory() -> anyhow::Result<Self> {
        let schema = build_schema();
        let index = Index::create_in_ram(schema.clone());
        Ok(Self {
            index,
            schema,
            writer: None,
        })
    }

    // ------------------------------------------------------------------------
    // Indexing
    // ------------------------------------------------------------------------

    /// Start a bulk indexing session.
    ///
    /// Call `add_node` repeatedly, then `commit`.
    pub fn start_indexing(&mut self) -> anyhow::Result<()> {
        let writer = self.index.writer(50_000_000)?;
        self.writer = Some(writer);
        Ok(())
    }

    /// Add a single node to the index.
    ///
    /// Panics if `start_indexing` was not called.
    pub fn add_node(&mut self, node: &CodeNode) -> anyhow::Result<()> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("indexing session not started"))?;

        let doc = node_to_document(node, &self.schema);
        writer.add_document(doc)?;
        Ok(())
    }

    /// Commit the current indexing session and make documents searchable.
    pub fn commit(&mut self) -> anyhow::Result<()> {
        let mut writer = self
            .writer
            .take()
            .ok_or_else(|| anyhow::anyhow!("no active indexing session"))?;
        writer.commit()?;
        Ok(())
    }

    /// Convenience: index a batch of nodes in one shot.
    pub fn index_nodes(&mut self, nodes: &[CodeNode]) -> anyhow::Result<()> {
        self.start_indexing()?;
        for node in nodes {
            self.add_node(node)?;
        }
        self.commit()?;
        Ok(())
    }

    // ------------------------------------------------------------------------
    // Search
    // ------------------------------------------------------------------------

    /// Execute a BM25 text search.
    ///
    /// `limit` controls the maximum number of results (default 50).
    pub fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<SearchHit>> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        let searcher: Searcher = reader.searcher();

        let name_field = self.schema.get_field("name")?;
        let file_field = self.schema.get_field("file_path")?;
        let kind_field = self.schema.get_field("kind")?;

        let parser = QueryParser::for_index(&self.index, vec![name_field, file_field, kind_field]);
        let parsed_query = parser.parse_query(query)?;

        let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(limit))?;

        let id_field = self.schema.get_field("id")?;
        let mut hits = Vec::with_capacity(top_docs.len());

        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;

            let node_id = doc
                .get_first(id_field)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let name = doc
                .get_first(name_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let file_path: std::path::PathBuf = doc
                .get_first(file_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .into();
            let kind_str = doc
                .get_first(kind_field)
                .and_then(|v| v.as_str())
                .unwrap_or("Function");
            let kind = parse_node_kind(kind_str).unwrap_or(NodeKind::Function);

            let node = CodeNode::new(
                cg_common::NodeId::new(node_id),
                kind,
                cg_common::NodeProperties::new(name, file_path),
            );

            hits.push(SearchHit {
                node,
                score: score as f64,
                source: SearchSource::Fts,
            });
        }

        Ok(hits)
    }

    /// Return the number of indexed documents.
    pub fn doc_count(&self) -> anyhow::Result<usize> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        Ok(reader.searcher().num_docs() as usize)
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn build_schema() -> Schema {
    let mut builder = Schema::builder();
    builder.add_u64_field("id", STORED);
    builder.add_text_field("name", TEXT | STORED);
    builder.add_text_field("file_path", TEXT | STORED);
    builder.add_text_field("kind", TEXT | STORED);
    builder.build()
}

fn node_to_document(node: &CodeNode, schema: &Schema) -> TantivyDocument {
    let mut doc = TantivyDocument::default();
    doc.add_u64(schema.get_field("id").unwrap(), node.id.0);
    doc.add_text(schema.get_field("name").unwrap(), &node.properties.name);
    doc.add_text(
        schema.get_field("file_path").unwrap(),
        node.properties.file_path.to_string_lossy(),
    );
    doc.add_text(schema.get_field("kind").unwrap(), node.kind.as_str());
    doc
}

fn parse_node_kind(s: &str) -> Option<NodeKind> {
    match s {
        "Project" => Some(NodeKind::Project),
        "Package" => Some(NodeKind::Package),
        "Module" => Some(NodeKind::Module),
        "Folder" => Some(NodeKind::Folder),
        "File" => Some(NodeKind::File),
        "Class" => Some(NodeKind::Class),
        "Function" => Some(NodeKind::Function),
        "Method" => Some(NodeKind::Method),
        "Variable" => Some(NodeKind::Variable),
        "Interface" => Some(NodeKind::Interface),
        "Enum" => Some(NodeKind::Enum),
        "Decorator" => Some(NodeKind::Decorator),
        "Import" => Some(NodeKind::Import),
        "Type" => Some(NodeKind::Type),
        "CodeElement" => Some(NodeKind::CodeElement),
        "Community" => Some(NodeKind::Community),
        "Process" => Some(NodeKind::Process),
        "Struct" => Some(NodeKind::Struct),
        "Macro" => Some(NodeKind::Macro),
        "Typedef" => Some(NodeKind::Typedef),
        "Union" => Some(NodeKind::Union),
        "Namespace" => Some(NodeKind::Namespace),
        "Trait" => Some(NodeKind::Trait),
        "Impl" => Some(NodeKind::Impl),
        "TypeAlias" => Some(NodeKind::TypeAlias),
        "Const" => Some(NodeKind::Const),
        "Static" => Some(NodeKind::Static),
        "Property" => Some(NodeKind::Property),
        "Record" => Some(NodeKind::Record),
        "Delegate" => Some(NodeKind::Delegate),
        "Annotation" => Some(NodeKind::Annotation),
        "Constructor" => Some(NodeKind::Constructor),
        "Template" => Some(NodeKind::Template),
        "Section" => Some(NodeKind::Section),
        "Route" => Some(NodeKind::Route),
        "Tool" => Some(NodeKind::Tool),
        _ => None,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cg_common::{NodeId, NodeProperties};

    fn make_node(id: u64, kind: NodeKind, name: &str, path: &str) -> CodeNode {
        CodeNode::new(NodeId::new(id), kind, NodeProperties::new(name, path))
    }

    #[test]
    fn index_and_search() {
        let mut idx = TantivyIndex::create_in_memory().unwrap();
        let nodes = vec![
            make_node(1, NodeKind::Function, "main", "/src/main.rs"),
            make_node(2, NodeKind::Struct, "UserConfig", "/src/config.rs"),
            make_node(3, NodeKind::Function, "parse_config", "/src/config.rs"),
        ];
        idx.index_nodes(&nodes).unwrap();

        let hits = idx.search("config", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|h| h.node.properties.name == "UserConfig"));
        assert!(
            hits.iter()
                .any(|h| h.node.properties.name == "parse_config")
        );
    }

    #[test]
    fn search_by_file_path() {
        let mut idx = TantivyIndex::create_in_memory().unwrap();
        let nodes = vec![
            make_node(1, NodeKind::Function, "foo", "/src/auth.rs"),
            make_node(2, NodeKind::Function, "bar", "/src/db.rs"),
        ];
        idx.index_nodes(&nodes).unwrap();

        let hits = idx.search("auth", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node.properties.name, "foo");
    }

    #[test]
    fn doc_count_matches_indexed() {
        let mut idx = TantivyIndex::create_in_memory().unwrap();
        let nodes = vec![
            make_node(1, NodeKind::Function, "a", "/a.rs"),
            make_node(2, NodeKind::Function, "b", "/b.rs"),
        ];
        idx.index_nodes(&nodes).unwrap();
        assert_eq!(idx.doc_count().unwrap(), 2);
    }
}
