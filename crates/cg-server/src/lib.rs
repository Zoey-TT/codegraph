//! CodeGraph — HTTP API server (axum).
//!
//! Minimal release endpoints: query, context, impact, graph/data.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, State},
    response::Json,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use cg_graph::{Direction, GraphStore, InMemoryGraphStore};
use cg_search::MemorySearcher;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    store: Arc<InMemoryGraphStore>,
}

impl AppState {
    pub fn new(store: InMemoryGraphStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }
}

/// API routes for the CodeGraph HTTP server.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/api/query", post(query))
        .route("/api/context/{symbol}", get(symbol_context))
        .route("/api/impact", post(impact))
        .route("/api/graph/data", get(graph_data))
        .with_state(state)
}

// ============================================================================
// Handlers
// ============================================================================

#[derive(Deserialize)]
struct QueryRequest {
    query: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Serialize)]
struct QueryResponse {
    hits: Vec<SearchHitResponse>,
    total: usize,
}

#[derive(Serialize)]
struct SearchHitResponse {
    id: u64,
    name: String,
    kind: String,
    file_path: String,
    score: f64,
}

async fn query(
    State(state): State<AppState>,
    Json(body): Json<QueryRequest>,
) -> Json<QueryResponse> {
    let searcher = MemorySearcher::new(state.store.as_ref());
    let kind_filter = body.kind.and_then(|k| parse_node_kind(&k));

    let hits = searcher
        .search_name(&body.query, kind_filter)
        .unwrap_or_default();
    let total = hits.len();
    let limit = body.limit.unwrap_or(20).min(100);

    let hits: Vec<_> = hits
        .into_iter()
        .take(limit)
        .map(|h| SearchHitResponse {
            id: h.node.id.0,
            name: h.node.properties.name,
            kind: format!("{:?}", h.node.kind),
            file_path: h.node.properties.file_path.to_string_lossy().to_string(),
            score: h.score,
        })
        .collect();

    Json(QueryResponse { hits, total })
}

async fn symbol_context(
    State(state): State<AppState>,
    Path(symbol): Path<String>,
) -> Json<serde_json::Value> {
    let searcher = MemorySearcher::new(state.store.as_ref());
    let hits = searcher.search_name(&symbol, None).unwrap_or_default();

    if let Some(hit) = hits.first() {
        match searcher.context(hit.node.id.0) {
            Ok(ctx) => {
                return Json(serde_json::json!({
                    "node": {
                        "id": ctx.node.id.0,
                        "name": ctx.node.properties.name,
                        "kind": format!("{:?}", ctx.node.kind),
                        "file_path": ctx.node.properties.file_path.to_string_lossy().to_string(),
                    },
                    "callers": ctx.callers,
                    "calls": ctx.calls,
                    "members": ctx.members,
                    "imports": ctx.imports,
                }));
            }
            Err(e) => {
                return Json(serde_json::json!({"error": format!("{}", e)}));
            }
        }
    }

    Json(serde_json::json!({"error": "not found"}))
}

#[derive(Deserialize)]
struct ImpactRequest {
    target: String,
    #[serde(default = "default_direction")]
    direction: String,
    #[serde(default = "default_max_depth")]
    max_depth: usize,
}

fn default_direction() -> String {
    "both".to_string()
}

fn default_max_depth() -> usize {
    5
}

#[derive(Serialize)]
struct ImpactResponse {
    target: String,
    direction: String,
    affected: Vec<AffectedSymbol>,
    total: usize,
}

#[derive(Serialize)]
struct AffectedSymbol {
    id: u64,
    name: String,
    kind: String,
    file_path: String,
    depth: usize,
}

async fn impact(
    State(state): State<AppState>,
    Json(body): Json<ImpactRequest>,
) -> Json<ImpactResponse> {
    let searcher = MemorySearcher::new(state.store.as_ref());

    let hits = searcher.search_name(&body.target, None).unwrap_or_default();
    let mut affected = Vec::new();

    if let Some(hit) = hits.first() {
        let start_id = hit.node.id.0;
        let mut visited = std::collections::HashSet::new();
        let mut raw = Vec::new();

        match body.direction.as_str() {
            "upstream" => {
                traverse_callers(&searcher, start_id, body.max_depth, &mut raw, &mut visited);
            }
            "downstream" => {
                traverse_callees(&searcher, start_id, body.max_depth, &mut raw, &mut visited);
            }
            _ => {
                traverse_callers(&searcher, start_id, body.max_depth, &mut raw, &mut visited);
                visited.clear();
                traverse_callees(&searcher, start_id, body.max_depth, &mut raw, &mut visited);
            }
        }

        for (depth, id) in raw {
            if let Ok(Some(node)) = state.store.get_node(&id.to_string()) {
                affected.push(AffectedSymbol {
                    id,
                    name: node.properties.name,
                    kind: format!("{:?}", node.kind),
                    file_path: node.properties.file_path.to_string_lossy().to_string(),
                    depth: body.max_depth - depth + 1,
                });
            }
        }
    }

    Json(ImpactResponse {
        target: body.target,
        direction: body.direction,
        total: affected.len(),
        affected,
    })
}

fn traverse_callers(
    searcher: &MemorySearcher,
    node_id: u64,
    max_depth: usize,
    out: &mut Vec<(usize, u64)>,
    visited: &mut std::collections::HashSet<u64>,
) {
    if max_depth == 0 || !visited.insert(node_id) {
        return;
    }
    if let Ok(ctx) = searcher.context(node_id) {
        for &caller in &ctx.callers {
            out.push((max_depth, caller));
            traverse_callers(searcher, caller, max_depth - 1, out, visited);
        }
    }
}

fn traverse_callees(
    searcher: &MemorySearcher,
    node_id: u64,
    max_depth: usize,
    out: &mut Vec<(usize, u64)>,
    visited: &mut std::collections::HashSet<u64>,
) {
    if max_depth == 0 || !visited.insert(node_id) {
        return;
    }
    if let Ok(ctx) = searcher.context(node_id) {
        for &callee in &ctx.calls {
            out.push((max_depth, callee));
            traverse_callees(searcher, callee, max_depth - 1, out, visited);
        }
    }
}

#[derive(Deserialize)]
struct GraphDataRequest {
    #[serde(default = "default_graph_limit")]
    limit: usize,
    #[serde(default)]
    kind: Option<String>,
}

fn default_graph_limit() -> usize {
    1000
}

async fn graph_data(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<GraphDataRequest>,
) -> Json<serde_json::Value> {
    let kind_filter = params.kind.as_deref().and_then(parse_node_kind);

    let nodes: Vec<_> = match state.store.all_nodes() {
        Ok(all) => all
            .into_iter()
            .filter(|n| kind_filter.is_none_or(|k| n.kind == k))
            .take(params.limit)
            .map(|n| {
                serde_json::json!({
                    "id": n.id.0,
                    "kind": format!("{:?}", n.kind),
                    "name": n.properties.name,
                    "file_path": n.properties.file_path.to_string_lossy().to_string(),
                    "language": n.properties.language.map(|l| format!("{:?}", l)),
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    // Collect edges for the nodes we returned
    let node_ids: std::collections::HashSet<u64> =
        nodes.iter().map(|n| n["id"].as_u64().unwrap()).collect();
    let mut edges = Vec::new();

    if let Ok(all_nodes) = state.store.all_nodes() {
        for node in all_nodes.into_iter().take(params.limit) {
            if let Ok(outgoing) =
                state
                    .store
                    .query_neighbors(&node.id.0.to_string(), Direction::Outgoing, None)
            {
                for edge in outgoing {
                    if node_ids.contains(&edge.source_id.0) && node_ids.contains(&edge.target_id.0)
                    {
                        edges.push(serde_json::json!({
                            "source": edge.source_id.0,
                            "target": edge.target_id.0,
                            "kind": format!("{:?}", edge.kind),
                            "confidence": edge.confidence,
                        }));
                    }
                }
            }
        }
    }

    Json(serde_json::json!({
        "nodes": nodes,
        "edges": edges,
        "limit": params.limit,
        "node_count": nodes.len(),
        "edge_count": edges.len(),
    }))
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
        _ => None,
    }
}

/// Start the HTTP server.
pub async fn serve(host: &str, port: u16, repo_path: &std::path::Path) -> anyhow::Result<()> {
    let codegraph_dir = repo_path.join(".codegraph");
    let store = InMemoryGraphStore::import_jsonl(&codegraph_dir)?;
    let state = AppState::new(store);

    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("CodeGraph server listening on http://{}", addr);
    axum::serve(listener, app(state)).await?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_state() -> AppState {
        AppState::new(InMemoryGraphStore::new())
    }

    #[tokio::test]
    async fn query_empty() {
        let app = app(test_state());
        let response = app
            .oneshot(
                Request::post("/api/query")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"query":"foo"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn context_not_found() {
        let app = app(test_state());
        let response = app
            .oneshot(
                Request::get("/api/context/missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
