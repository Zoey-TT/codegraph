//! CodeGraph — MCP server implementation and tool definitions.

/// MCP tool definitions.
pub mod tools;

/// Resource URI definitions.
pub mod resources;

/// Server configuration and lifecycle.
pub mod server;

/// MCP client setup (Claude Desktop, Cursor, Windsurf, Cline).
pub mod setup;

pub use server::{CodeGraphMcpServer, run_stdio_server};
pub use setup::{
    McpClient, SetupAction, SetupResult, detect_clients, generate_manual_config, setup_all_clients,
    setup_client,
};
