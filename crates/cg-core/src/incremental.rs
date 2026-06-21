//! Incremental indexing support.
//!
//! Computes git diffs, content hashes, and delta updates so that only
//! changed files need to be re-parsed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::KnowledgeGraph;

// ============================================================================
// FileDelta
// ============================================================================

/// Set of files that changed between two git commits.
#[derive(Debug, Clone, Default)]
pub struct FileDelta {
    /// New files.
    pub added: Vec<PathBuf>,
    /// Files whose contents changed.
    pub modified: Vec<PathBuf>,
    /// Files that no longer exist.
    pub deleted: Vec<PathBuf>,
    /// Files that were renamed (old_path, new_path).
    pub renamed: Vec<(PathBuf, PathBuf)>,
}

impl FileDelta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.modified.is_empty()
            && self.deleted.is_empty()
            && self.renamed.is_empty()
    }

    /// All paths that need to be re-parsed (added + modified + renamed targets).
    pub fn files_to_parse(&self) -> Vec<&Path> {
        let mut v: Vec<_> = self.added.iter().map(|p| p.as_path()).collect();
        v.extend(self.modified.iter().map(|p| p.as_path()));
        v.extend(self.renamed.iter().map(|(_, new)| new.as_path()));
        v
    }

    /// All paths whose old nodes should be removed (modified + deleted + renamed sources).
    pub fn files_to_remove(&self) -> Vec<&Path> {
        let mut v: Vec<_> = self.modified.iter().map(|p| p.as_path()).collect();
        v.extend(self.deleted.iter().map(|p| p.as_path()));
        v.extend(self.renamed.iter().map(|(old, _)| old.as_path()));
        v
    }
}

// ============================================================================
// Git diff
// ============================================================================

/// Compute the set of changed files between `old_commit` and `new_commit`.
///
/// `repo_path` should point to the repository root (or anywhere inside it).
pub fn compute_file_delta(
    repo_path: &Path,
    old_commit: &str,
    new_commit: &str,
) -> anyhow::Result<FileDelta> {
    let repo = git2::Repository::discover(repo_path)?;
    let old_oid = repo.revparse_single(old_commit)?.id();
    let new_oid = repo.revparse_single(new_commit)?.id();

    let old_tree = repo.find_commit(old_oid)?.tree()?;
    let new_tree = repo.find_commit(new_oid)?.tree()?;

    let mut delta = FileDelta::default();
    let diff = repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)?;

    diff.foreach(
        &mut |file, _progress| {
            let old_path = file.old_file().path().map(|p| p.to_path_buf());
            let new_path = file.new_file().path().map(|p| p.to_path_buf());

            match file.status() {
                git2::Delta::Added => {
                    if let Some(p) = new_path {
                        delta.added.push(p);
                    }
                }
                git2::Delta::Deleted => {
                    if let Some(p) = old_path {
                        delta.deleted.push(p);
                    }
                }
                git2::Delta::Modified | git2::Delta::Conflicted => {
                    if let Some(p) = new_path {
                        delta.modified.push(p);
                    }
                }
                git2::Delta::Renamed | git2::Delta::Copied | git2::Delta::Typechange => {
                    if let (Some(old), Some(new)) = (old_path, new_path) {
                        delta.renamed.push((old, new));
                    }
                }
                _ => {}
            }
            true
        },
        None,
        None,
        None,
    )?;

    Ok(delta)
}

// ============================================================================
// Content hashes
// ============================================================================

/// Persistent cache of per-file SHA-256 content hashes.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ContentHashes {
    pub hashes: HashMap<PathBuf, String>,
}

impl ContentHashes {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let this: Self = serde_json::from_str(&content)?;
        Ok(this)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Compute SHA-256 hash of a file's contents.
    pub fn hash_file(path: &Path) -> anyhow::Result<String> {
        use sha2::Digest;
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        let mut hasher = sha2::Sha256::new();
        let mut buffer = [0u8; 8192];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    /// Return paths whose hash differs from the cache (or are not cached).
    pub fn changed_files<'a>(&self, paths: &'a [PathBuf]) -> Vec<&'a PathBuf> {
        paths
            .iter()
            .filter(|p| match self.hashes.get(*p) {
                Some(cached) => match Self::hash_file(p) {
                    Ok(current) => &current != cached,
                    Err(_) => true,
                },
                None => true,
            })
            .collect()
    }
}

// ============================================================================
// Graph mutation helpers
// ============================================================================

/// Remove all nodes (and their edges) that belong to a given file path.
pub fn remove_file_nodes(graph: &KnowledgeGraph, file_path: &Path) {
    // DashMap iteration: collect node ids first to avoid holding locks during mutation
    let to_remove: Vec<_> = graph
        .file_index
        .get(file_path)
        .map(|entry| entry.value().clone())
        .unwrap_or_default();

    for node_id in to_remove {
        graph.remove_node(&node_id);
    }

    // Also remove the file node itself if it exists
    // (File nodes are tracked in the graph by their own id, not via file_index)
    // We rely on the caller to also remove folder nodes if folders were deleted.
}

/// Remove all nodes for a set of file paths.
pub fn remove_files_nodes(graph: &KnowledgeGraph, file_paths: &[&Path]) {
    for path in file_paths {
        remove_file_nodes(graph, path);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_delta_empty() {
        let d = FileDelta::default();
        assert!(d.is_empty());
    }

    #[test]
    fn file_delta_collect() {
        let d = FileDelta {
            added: vec![PathBuf::from("a.rs")],
            modified: vec![PathBuf::from("b.rs")],
            deleted: vec![PathBuf::from("c.rs")],
            renamed: vec![(PathBuf::from("d.rs"), PathBuf::from("e.rs"))],
        };
        assert_eq!(d.files_to_parse().len(), 3);
        assert_eq!(d.files_to_remove().len(), 3);
    }

    #[test]
    fn content_hashes_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hashes.json");
        let mut hashes = ContentHashes::default();
        hashes
            .hashes
            .insert(PathBuf::from("a.rs"), "abc".to_string());
        hashes.save(&path).unwrap();
        let loaded = ContentHashes::load(&path).unwrap();
        assert_eq!(loaded.hashes[&PathBuf::from("a.rs")], "abc");
    }
}
