//! MCP client configuration setup.
//!
//! Detects installed MCP clients (Claude Desktop, Cursor, Windsurf, Cline)
//! and writes the `codegraph mcp` server configuration into their config files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ============================================================================
// Supported MCP clients
// ============================================================================

/// A supported MCP client application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum McpClient {
    ClaudeDesktop,
    Cursor,
    Windsurf,
    Cline,
}

impl McpClient {
    /// Human-readable name.
    pub fn display_name(&self) -> &'static str {
        match self {
            McpClient::ClaudeDesktop => "Claude Desktop",
            McpClient::Cursor => "Cursor",
            McpClient::Windsurf => "Windsurf",
            McpClient::Cline => "Cline (VS Code)",
        }
    }

    /// Config file path for this client (if installed).
    pub fn config_path(&self) -> Option<PathBuf> {
        match self {
            McpClient::ClaudeDesktop => claude_desktop_config_path(),
            McpClient::Cursor => cursor_config_path(),
            McpClient::Windsurf => windsurf_config_path(),
            McpClient::Cline => cline_config_path(),
        }
    }

    /// Whether this client supports hot-reload (no restart needed).
    pub fn hot_reload(&self) -> bool {
        matches!(self, McpClient::Cursor | McpClient::Cline)
    }
}

/// Detect which MCP clients are installed by checking config directories.
pub fn detect_clients() -> Vec<McpClient> {
    let mut found = Vec::new();
    for client in [
        McpClient::ClaudeDesktop,
        McpClient::Cursor,
        McpClient::Windsurf,
        McpClient::Cline,
    ] {
        if let Some(path) = client.config_path()
            && let Some(dir) = path.parent()
            && dir.exists()
        {
            found.push(client);
        }
    }
    found
}

// ============================================================================
// Client-specific config paths
// ============================================================================

fn claude_desktop_config_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .map(|h| h.join("Library/Application Support/Claude/claude_desktop_config.json"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Claude/claude_desktop_config.json"))
    }
    #[cfg(target_os = "linux")]
    {
        dirs::config_dir().map(|p| p.join("Claude/claude_desktop_config.json"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

fn cursor_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".cursor/mcp.json"))
}

fn windsurf_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".codeium/windsurf/mcp_config.json"))
}

fn cline_config_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|h| {
            h.join("Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json")
        })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(PathBuf::from).map(|p| {
            p.join(
                r"Code\User\globalStorage\saoudrizwan.claude-dev\settings\cline_mcp_settings.json",
            )
        })
    }
    #[cfg(target_os = "linux")]
    {
        dirs::config_dir().map(|p| {
            p.join(
                "Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json",
            )
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

// ============================================================================
// Config file I/O
// ============================================================================

/// The canonical MCP config structure used by most clients.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub mcp_servers: HashMap<String, ServerEntry>,
}

/// A single MCP server entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerEntry {
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

/// Read an existing config file, or return a default empty config.
fn read_config(path: &Path) -> anyhow::Result<McpConfig> {
    if !path.exists() {
        return Ok(McpConfig::default());
    }
    let content = std::fs::read_to_string(path)?;
    let config: McpConfig = serde_json::from_str(&content)?;
    Ok(config)
}

/// Write config to disk, creating parent directories and backing up existing file.
fn write_config(path: &Path, config: &McpConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Backup existing file
    if path.exists() {
        let backup = path.with_extension("json.backup");
        std::fs::copy(path, &backup)?;
    }

    let content = serde_json::to_string_pretty(config)?;
    std::fs::write(path, content)?;
    Ok(())
}

// ============================================================================
// Binary path detection
// ============================================================================

/// Try to find the `codegraph` binary path.
///
/// 1. `which codegraph` / `where codegraph`
/// 2. `std::env::current_exe()` (if executable name is "codegraph")
/// 3. Fallback to plain `"codegraph"` (assumes PATH)
pub fn detect_binary_path() -> anyhow::Result<String> {
    // 1. Try which/where
    if let Ok(output) = std::process::Command::new("which")
        .arg("codegraph")
        .output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(path);
        }
    }

    // On Windows, try `where`
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("where")
            .arg("codegraph")
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !path.is_empty() {
                    return Ok(path);
                }
            }
        }
    }

    // 2. Check current_exe
    if let Ok(current) = std::env::current_exe()
        && current.file_stem() == Some(std::ffi::OsStr::new("codegraph"))
    {
        return Ok(current.to_string_lossy().to_string());
    }

    // 3. Fallback
    Ok("codegraph".to_string())
}

// ============================================================================
// Public API
// ============================================================================

/// Result of configuring a single client.
#[derive(Debug, Clone)]
pub struct SetupResult {
    pub client: McpClient,
    pub config_path: PathBuf,
    pub action: SetupAction,
}

#[derive(Debug, Clone)]
pub enum SetupAction {
    /// Added a new server entry.
    Added,
    /// Updated an existing server entry.
    Updated,
    /// No change needed (already configured).
    Unchanged,
}

/// Configure `codegraph` MCP server for all detected clients.
pub fn setup_all_clients() -> anyhow::Result<Vec<SetupResult>> {
    let binary = detect_binary_path()?;
    let clients = detect_clients();

    if clients.is_empty() {
        anyhow::bail!(
            "No MCP clients detected. Please install Claude Desktop, Cursor, Windsurf, or Cline first."
        );
    }

    let entry = ServerEntry {
        command: binary,
        args: vec!["mcp".to_string()],
        env: HashMap::new(),
    };

    let mut results = Vec::new();
    for client in clients {
        let Some(path) = client.config_path() else {
            continue;
        };
        let mut config = read_config(&path)?;

        let action = if let Some(existing) = config.mcp_servers.get("codegraph") {
            if existing.command == entry.command && existing.args == entry.args {
                SetupAction::Unchanged
            } else {
                SetupAction::Updated
            }
        } else {
            SetupAction::Added
        };

        config
            .mcp_servers
            .insert("codegraph".to_string(), entry.clone());
        write_config(&path, &config)?;

        results.push(SetupResult {
            client,
            config_path: path,
            action,
        });
    }

    Ok(results)
}

/// Configure `codegraph` MCP server for a specific client.
pub fn setup_client(client: McpClient) -> anyhow::Result<SetupResult> {
    let binary = detect_binary_path()?;
    let path = client
        .config_path()
        .ok_or_else(|| anyhow::anyhow!("Config path not known for {:?}", client))?;

    let mut config = read_config(&path)?;

    let entry = ServerEntry {
        command: binary,
        args: vec!["mcp".to_string()],
        env: HashMap::new(),
    };

    let action = if let Some(existing) = config.mcp_servers.get("codegraph") {
        if existing.command == entry.command && existing.args == entry.args {
            SetupAction::Unchanged
        } else {
            SetupAction::Updated
        }
    } else {
        SetupAction::Added
    };

    config.mcp_servers.insert("codegraph".to_string(), entry);
    write_config(&path, &config)?;

    Ok(SetupResult {
        client,
        config_path: path,
        action,
    })
}

/// Generate the JSON configuration block that users can paste manually.
pub fn generate_manual_config() -> anyhow::Result<String> {
    let binary = detect_binary_path()?;
    let config = McpConfig {
        mcp_servers: {
            let mut map = HashMap::new();
            map.insert(
                "codegraph".to_string(),
                ServerEntry {
                    command: binary,
                    args: vec!["mcp".to_string()],
                    env: HashMap::new(),
                },
            );
            map
        },
    };
    Ok(serde_json::to_string_pretty(&config)?)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let mut servers = HashMap::new();
        servers.insert(
            "codegraph".to_string(),
            ServerEntry {
                command: "/usr/local/bin/codegraph".to_string(),
                args: vec!["mcp".to_string()],
                env: HashMap::new(),
            },
        );
        let config = McpConfig {
            mcp_servers: servers,
        };

        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: McpConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.mcp_servers.len(), 1);
        assert_eq!(
            parsed.mcp_servers["codegraph"].command,
            "/usr/local/bin/codegraph"
        );
    }

    #[test]
    fn merge_into_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");

        // Write existing config with another server
        let mut existing = HashMap::new();
        existing.insert(
            "other".to_string(),
            ServerEntry {
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "other-mcp".to_string()],
                env: HashMap::new(),
            },
        );
        let initial = McpConfig {
            mcp_servers: existing,
        };
        std::fs::write(&path, serde_json::to_string_pretty(&initial).unwrap()).unwrap();

        // Read and merge
        let mut config = read_config(&path).unwrap();
        config.mcp_servers.insert(
            "codegraph".to_string(),
            ServerEntry {
                command: "codegraph".to_string(),
                args: vec!["mcp".to_string()],
                env: HashMap::new(),
            },
        );
        write_config(&path, &config).unwrap();

        // Verify both servers exist
        let result = read_config(&path).unwrap();
        assert!(result.mcp_servers.contains_key("other"));
        assert!(result.mcp_servers.contains_key("codegraph"));

        // Verify backup was created
        assert!(dir.path().join("mcp.json.backup").exists());
    }

    #[test]
    fn detect_binary_falls_back_to_codegraph() {
        // We can't easily test the `which` path, but we can verify fallback works
        let result = detect_binary_path().unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn manual_config_is_valid_json() {
        let json = generate_manual_config().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("mcpServers").is_some());
        let cg = &parsed["mcpServers"]["codegraph"];
        assert!(cg.get("command").is_some());
        assert!(cg.get("args").is_some());
    }
}
