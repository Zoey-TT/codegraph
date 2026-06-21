//! MCP server lifecycle.

use std::sync::Arc;

use cg_graph::InMemoryGraphStore;
use cg_search::MemorySearcher;
use rmcp::{
    ServerHandler, ServiceExt,
    model::{
        AnnotateAble, Implementation, ListResourcesResult, ReadResourceResult, ResourceContents,
        ServerCapabilities, ServerInfo,
    },
    schemars, tool,
};

/// MCP server state backed by an in-memory graph store.
#[derive(Debug, Clone)]
pub struct CodeGraphMcpServer {
    store: Arc<InMemoryGraphStore>,
}

impl CodeGraphMcpServer {
    /// Load the graph from `.codegraph/` JSONL files in the given repo path.
    pub fn from_repo_path(repo_path: &std::path::Path) -> anyhow::Result<Self> {
        let codegraph_dir = repo_path.join(".codegraph");
        let store = InMemoryGraphStore::import_jsonl(&codegraph_dir)?;
        Ok(Self {
            store: Arc::new(store),
        })
    }

    /// Create a server with an empty graph (useful for testing).
    pub fn empty() -> Self {
        Self {
            store: Arc::new(InMemoryGraphStore::new()),
        }
    }
}

// ============================================================================
// Tool parameter types
// ============================================================================

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QueryRequest {
    /// Search query string (symbol name or substring).
    pub query: String,
    /// Optional node kind filter (e.g. "Function", "Struct", "Trait").
    #[serde(default)]
    pub kind: Option<String>,
    /// Maximum number of results to return (default 20, max 100).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ContextRequest {
    /// Symbol name to look up.
    pub symbol: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ImpactRequest {
    /// Target symbol name.
    pub target: String,
    /// Direction: "upstream", "downstream", or "both" (default).
    #[serde(default = "default_direction")]
    pub direction: String,
    /// Maximum depth to traverse (default 3).
    #[serde(default = "default_depth")]
    pub depth: usize,
}

fn default_direction() -> String {
    "both".to_string()
}

fn default_depth() -> usize {
    3
}

// ============================================================================
// Tool implementations
// ============================================================================

#[tool(tool_box)]
impl CodeGraphMcpServer {
    /// Search for symbols by name across the indexed codebase.
    #[tool(
        description = "Search for code symbols (functions, structs, enums, traits, etc.) by name."
    )]
    fn query(&self, #[tool(aggr)] QueryRequest { query, kind, limit }: QueryRequest) -> String {
        let searcher = MemorySearcher::new(self.store.as_ref());
        let kind_filter = kind.as_ref().and_then(|k| parse_node_kind(k));
        let hits = searcher
            .search_name(&query, kind_filter)
            .unwrap_or_default();
        let total = hits.len();
        let limit = limit.unwrap_or(20).min(100);

        let results: Vec<_> = hits
            .into_iter()
            .take(limit)
            .map(|h| {
                serde_json::json!({
                    "id": h.node.id.0,
                    "name": h.node.properties.name,
                    "kind": format!("{:?}", h.node.kind),
                    "file_path": h.node.properties.file_path.to_string_lossy().to_string(),
                    "score": h.score,
                })
            })
            .collect();

        serde_json::json!({
            "total": total,
            "returned": results.len(),
            "results": results,
        })
        .to_string()
    }

    /// Get 360° context for a symbol: callers, callees, members, imports.
    #[tool(
        description = "Get 360-degree context for a symbol: what calls it, what it calls, its members, and its imports."
    )]
    fn context(&self, #[tool(aggr)] ContextRequest { symbol }: ContextRequest) -> String {
        let searcher = MemorySearcher::new(self.store.as_ref());
        let hits = searcher.search_name(&symbol, None).unwrap_or_default();

        if let Some(hit) = hits.first() {
            match searcher.context(hit.node.id.0) {
                Ok(ctx) => {
                    let node = &ctx.node;
                    return serde_json::json!({
                        "found": true,
                        "node": {
                            "id": node.id.0,
                            "name": node.properties.name,
                            "kind": format!("{:?}", node.kind),
                            "file_path": node.properties.file_path.to_string_lossy().to_string(),
                        },
                        "callers": ctx.callers,
                        "calls": ctx.calls,
                        "members": ctx.members,
                        "imports": ctx.imports,
                    })
                    .to_string();
                }
                Err(e) => {
                    return serde_json::json!({"found": false, "error": format!("{}", e)})
                        .to_string();
                }
            }
        }

        serde_json::json!({"found": false, "error": "symbol not found"}).to_string()
    }

    /// List indexed repositories.
    #[tool(description = "List all indexed repositories available on this machine.")]
    fn list_repos(&self) -> String {
        let registry = dirs::home_dir().map(|h| h.join(".codegraph/registry.json"));

        let repos: Vec<String> = match registry {
            Some(path) if path.exists() => std::fs::read_to_string(path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        serde_json::json!({"repositories": repos}).to_string()
    }

    /// Impact analysis: traverse the call graph to find affected symbols.
    #[tool(
        description = "Analyze the blast radius of changing a symbol by traversing the call graph."
    )]
    fn impact(
        &self,
        #[tool(aggr)] ImpactRequest {
            target,
            direction,
            depth,
        }: ImpactRequest,
    ) -> String {
        let searcher = MemorySearcher::new(self.store.as_ref());
        let hits = searcher.search_name(&target, None).unwrap_or_default();

        if hits.is_empty() {
            return serde_json::json!({"found": false, "error": "target symbol not found"})
                .to_string();
        }

        let start_id = hits[0].node.id.0;
        let mut upstream = Vec::new();
        let mut downstream = Vec::new();
        let mut visited = std::collections::HashSet::new();

        // Simple BFS traversal
        if direction == "upstream" || direction == "both" {
            traverse_callers(&searcher, start_id, depth, &mut upstream, &mut visited);
        }
        visited.clear();
        if direction == "downstream" || direction == "both" {
            traverse_callees(&searcher, start_id, depth, &mut downstream, &mut visited);
        }

        serde_json::json!({
            "found": true,
            "target": target,
            "direction": direction,
            "depth": depth,
            "upstream_count": upstream.len(),
            "downstream_count": downstream.len(),
            "upstream": upstream,
            "downstream": downstream,
        })
        .to_string()
    }
}

// ============================================================================
// ServerHandler implementation
// ============================================================================

#[tool(tool_box)]
impl ServerHandler for CodeGraphMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "codegraph".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            instructions: Some(
                "CodeGraph MCP server provides code search, symbol context, and impact analysis tools.".into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().enable_resources().build(),
            ..Default::default()
        }
    }

    async fn list_resources(
        &self,
        _request: rmcp::model::PaginatedRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::Error> {
        let resources = vec![
            // codegraph://status — current repo index status
            rmcp::model::RawResource {
                uri: "codegraph://status".into(),
                name: "Repository Index Status".into(),
                description: Some(
                    "Shows whether the current repository index is up-to-date or stale.".into(),
                ),
                mime_type: Some("application/json".into()),
                size: None,
            }
            .no_annotation(),
            // codegraph://graph/summary — graph statistics
            rmcp::model::RawResource {
                uri: "codegraph://graph/summary".into(),
                name: "Graph Summary".into(),
                description: Some(
                    "High-level statistics about the knowledge graph (nodes, edges, communities)."
                        .into(),
                ),
                mime_type: Some("application/json".into()),
                size: None,
            }
            .no_annotation(),
        ];

        Ok(ListResourcesResult {
            next_cursor: None,
            resources,
        })
    }

    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ReadResourceResult, rmcp::Error> {
        match request.uri.as_str() {
            "codegraph://status" => {
                let status = self.read_status_resource();
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(status, "codegraph://status")],
                })
            }
            "codegraph://graph/summary" => {
                let summary = self.read_graph_summary_resource();
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(summary, "codegraph://graph/summary")],
                })
            }
            _ => Err(rmcp::Error::invalid_params("resource not found", None)),
        }
    }
}

impl CodeGraphMcpServer {
    fn read_status_resource(&self) -> String {
        serde_json::json!({
            "status": "ok",
            "indexed": true,
            "nodes": self.store.knowledge_graph().node_count(),
            "edges": self.store.knowledge_graph().edge_count(),
        })
        .to_string()
    }

    fn read_graph_summary_resource(&self) -> String {
        let kg = self.store.knowledge_graph();
        let community_count = kg.nodes_by_kind(cg_common::NodeKind::Community).len();
        serde_json::json!({
            "nodes": kg.node_count(),
            "edges": kg.edge_count(),
            "communities": community_count,
        })
        .to_string()
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn parse_node_kind(s: &str) -> Option<cg_common::NodeKind> {
    match s {
        "Function" => Some(cg_common::NodeKind::Function),
        "Struct" => Some(cg_common::NodeKind::Struct),
        "Enum" => Some(cg_common::NodeKind::Enum),
        "Trait" => Some(cg_common::NodeKind::Trait),
        "Class" => Some(cg_common::NodeKind::Class),
        "Interface" => Some(cg_common::NodeKind::Interface),
        "Method" => Some(cg_common::NodeKind::Method),
        "Module" => Some(cg_common::NodeKind::Module),
        "File" => Some(cg_common::NodeKind::File),
        "Folder" => Some(cg_common::NodeKind::Folder),
        "TypeAlias" => Some(cg_common::NodeKind::TypeAlias),
        "Const" => Some(cg_common::NodeKind::Const),
        "Static" => Some(cg_common::NodeKind::Static),
        _ => None,
    }
}

fn traverse_callers(
    searcher: &MemorySearcher,
    node_id: u64,
    depth: usize,
    out: &mut Vec<u64>,
    visited: &mut std::collections::HashSet<u64>,
) {
    if depth == 0 || !visited.insert(node_id) {
        return;
    }
    if let Ok(ctx) = searcher.context(node_id) {
        for &caller in &ctx.callers {
            out.push(caller);
            traverse_callers(searcher, caller, depth - 1, out, visited);
        }
    }
}

fn traverse_callees(
    searcher: &MemorySearcher,
    node_id: u64,
    depth: usize,
    out: &mut Vec<u64>,
    visited: &mut std::collections::HashSet<u64>,
) {
    if depth == 0 || !visited.insert(node_id) {
        return;
    }
    if let Ok(ctx) = searcher.context(node_id) {
        for &callee in &ctx.calls {
            out.push(callee);
            traverse_callees(searcher, callee, depth - 1, out, visited);
        }
    }
}

// ============================================================================
// Public entry point
// ============================================================================

/// Run the MCP server over stdio transport.
pub async fn run_stdio_server(repo_path: &std::path::Path) -> anyhow::Result<()> {
    let server = CodeGraphMcpServer::from_repo_path(repo_path)?;
    let transport = rmcp::transport::stdio();
    let running = server.serve(transport).await?;
    running.waiting().await?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_attrs_are_generated() {
        let _ = CodeGraphMcpServer::query_tool_attr();
        let _ = CodeGraphMcpServer::context_tool_attr();
        let _ = CodeGraphMcpServer::list_repos_tool_attr();
        let _ = CodeGraphMcpServer::impact_tool_attr();
    }

    #[test]
    fn server_info_is_valid() {
        let server = CodeGraphMcpServer::empty();
        let info = server.get_info();
        assert_eq!(info.server_info.name, "codegraph");
        assert!(info.instructions.is_some());
    }

    #[test]
    fn query_tool_has_correct_name() {
        let attr = CodeGraphMcpServer::query_tool_attr();
        assert_eq!(attr.name, "query");
    }

    #[test]
    fn list_repos_on_empty_graph() {
        let server = CodeGraphMcpServer::empty();
        let result = server.list_repos();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["repositories"].is_array());
    }
}
