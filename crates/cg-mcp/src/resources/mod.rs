//! MCP resource URIs.

/// All indexed repositories.
pub const REPOS: &str = "codegraph://repos";

/// Repository statistics + staleness check.
pub fn repo_context(name: &str) -> String {
    format!("codegraph://repo/{}/context", name)
}

/// Functional clusters for a repository.
pub fn repo_clusters(name: &str) -> String {
    format!("codegraph://repo/{}/clusters", name)
}

/// Execution flow list for a repository.
pub fn repo_processes(name: &str) -> String {
    format!("codegraph://repo/{}/processes", name)
}

/// Graph schema definition for a repository.
pub fn repo_schema(name: &str) -> String {
    format!("codegraph://repo/{}/schema", name)
}
