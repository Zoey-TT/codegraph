//! MCP tool name constants.
//!
//! The minimal release exposes only the tools that are actually implemented:
//! query, context, impact, and list_repos.

/// Search for code symbols by name.
pub const QUERY: &str = "query";

/// Get 360-degree context for a symbol.
pub const CONTEXT: &str = "context";

/// Analyze the blast radius of changing a symbol.
pub const IMPACT: &str = "impact";

/// List all indexed repositories.
pub const LIST_REPOS: &str = "list_repos";
