//! Global repository registry management.
//!
//! Stores a list of all indexed repositories in `~/.codegraph/registry.json`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A single entry in the global registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegistryEntry {
    /// Human-readable name (directory name).
    pub name: String,
    /// Absolute path to the repository.
    pub path: PathBuf,
    /// Unix timestamp of the last successful index.
    pub last_indexed_at: i64,
    /// Number of nodes in the last index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_count: Option<usize>,
    /// Number of edges in the last index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_count: Option<usize>,
}

/// The global registry file path.
///
/// Honors the `CODEGRAPH_REGISTRY` environment variable for testing or custom
/// setups; otherwise defaults to `~/.codegraph/registry.json`.
pub fn registry_path() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("CODEGRAPH_REGISTRY") {
        return Some(PathBuf::from(env_path));
    }
    dirs::home_dir().map(|h| h.join(".codegraph/registry.json"))
}

/// Load the registry from disk.
pub fn load_registry() -> anyhow::Result<Vec<RegistryEntry>> {
    let path = match registry_path() {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)?;
    let entries: Vec<RegistryEntry> = serde_json::from_str(&content)?;
    Ok(entries)
}

/// Save the registry to disk.
pub fn save_registry(entries: &[RegistryEntry]) -> anyhow::Result<()> {
    let path = match registry_path() {
        Some(p) => p,
        None => anyhow::bail!("Could not determine home directory"),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(entries)?;
    std::fs::write(&path, content)?;
    Ok(())
}

/// Register or update a repository in the global registry.
pub fn register_repo(
    repo_path: &Path,
    node_count: Option<usize>,
    edge_count: Option<usize>,
) -> anyhow::Result<()> {
    let name = repo_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let abs_path = std::fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let mut entries = load_registry()?;

    // Remove existing entry for this path (if any)
    entries.retain(|e| e.path != abs_path);

    entries.push(RegistryEntry {
        name,
        path: abs_path,
        last_indexed_at: now,
        node_count,
        edge_count,
    });

    save_registry(&entries)?;
    Ok(())
}

/// Remove a repository from the global registry.
pub fn unregister_repo(repo_path: &Path) -> anyhow::Result<()> {
    let abs_path = std::fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let mut entries = load_registry()?;
    let before = entries.len();
    entries.retain(|e| e.path != abs_path);
    if entries.len() < before {
        save_registry(&entries)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_registry() {
        let _tmp = tempfile::tempdir().unwrap();
        // Override registry path via env or just test the in-memory logic
        let entries = vec![RegistryEntry {
            name: "foo".into(),
            path: PathBuf::from("/tmp/foo"),
            last_indexed_at: 1234567890,
            node_count: Some(100),
            edge_count: Some(200),
        }];
        let json = serde_json::to_string_pretty(&entries).unwrap();
        let parsed: Vec<RegistryEntry> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entries);
    }
}
