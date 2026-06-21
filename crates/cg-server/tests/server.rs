//! Integration tests for the HTTP API server.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cg_graph::InMemoryGraphStore;
use cg_server::AppState;
use tower::ServiceExt;

fn test_app() -> axum::Router {
    cg_server::app(AppState::new(InMemoryGraphStore::new()))
}

#[tokio::test]
async fn query_returns_empty_hits() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::post("/api/query")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"query":"nonexistent"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn context_not_found_for_missing_symbol() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::get("/api/context/DefinitelyNotARealSymbol")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn impact_returns_empty_for_missing_symbol() {
    let app = test_app();
    let response = app
        .oneshot(
            Request::post("/api/impact")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"target":"missing"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn graph_data_returns_empty() {
    let app = test_app();
    let response = app
        .oneshot(Request::get("/api/graph/data").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
