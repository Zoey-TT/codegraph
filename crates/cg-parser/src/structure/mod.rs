//! Build Folder/File hierarchy from scanned files.
//!
//! Generates `Folder` / `File` nodes and `CONTAINS` edges that mirror
//! the repository's directory tree.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cg_common::{CodeEdge, CodeNode, EdgeKind, NodeId, NodeKind, NodeProperties};

use crate::scanner::FileInfo;

/// Output of the structure phase.
#[derive(Debug, Clone, Default)]
pub struct StructureOutput {
    /// All file paths discovered (relative).
    pub all_paths: HashSet<PathBuf>,
    /// All folder paths discovered (relative).
    pub all_folders: HashSet<PathBuf>,
    /// Folder nodes created.
    pub folder_nodes: Vec<CodeNode>,
    /// File nodes created.
    pub file_nodes: Vec<CodeNode>,
    /// CONTAINS edges (parent → child).
    pub contains_edges: Vec<CodeEdge>,
    /// Root folder path (if any).
    pub root: Option<PathBuf>,
}

/// Node ID generator based on label + path (deterministic).
pub fn make_node_id(label: &str, path: &Path) -> NodeId {
    use rustc_hash::FxHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    label.hash(&mut hasher);
    path.to_string_lossy().hash(&mut hasher);
    NodeId::new(hasher.finish())
}

/// Build structure nodes and CONTAINS edges from file scan results.
///
/// All paths are stored relative to `root` (or absolute if the file path
/// is not under `root`).
pub fn build_structure(files: &[FileInfo], root: &Path) -> StructureOutput {
    let mut output = StructureOutput {
        root: Some(root.to_path_buf()),
        ..StructureOutput::default()
    };

    // Map from relative path → node id
    let mut path_to_id: std::collections::HashMap<PathBuf, NodeId> =
        std::collections::HashMap::new();

    // Collect all unique folder paths that need nodes
    let mut folder_paths: HashSet<PathBuf> = HashSet::new();
    for file in files {
        output.all_paths.insert(file.relative_path.clone());

        if let Some(parent) = file.relative_path.parent() {
            let mut current = PathBuf::new();
            for component in parent.components() {
                current.push(component);
                folder_paths.insert(current.clone());
            }
        }
    }

    // Create Folder nodes (sorted for deterministic order)
    let mut sorted_folders: Vec<_> = folder_paths.into_iter().collect();
    sorted_folders.sort();

    for folder_path in sorted_folders {
        let id = make_node_id("Folder", &folder_path);
        let name = folder_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| folder_path.to_string_lossy().to_string());
        let node = CodeNode::new(
            id,
            NodeKind::Folder,
            NodeProperties::new(name, folder_path.clone()),
        );
        path_to_id.insert(folder_path.clone(), id);
        output.all_folders.insert(folder_path);
        output.folder_nodes.push(node);
    }

    // Create File nodes and CONTAINS edges
    let mut sorted_files = files.to_vec();
    sorted_files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    for file in &sorted_files {
        let id = make_node_id("File", &file.relative_path);
        let name = file
            .relative_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let mut props = NodeProperties::new(name, file.relative_path.clone());
        props.language = file.language;
        let node = CodeNode::new(id, NodeKind::File, props);
        path_to_id.insert(file.relative_path.clone(), id);
        output.file_nodes.push(node);

        // Parent folder
        if let Some(parent) = file.relative_path.parent()
            && let Some(&parent_id) = path_to_id.get(parent)
        {
            let edge_id = make_node_id("CONTAINS", &file.relative_path);
            let edge = CodeEdge::new(
                edge_id,
                parent_id,
                id,
                EdgeKind::Contains,
                1.0,
                "directory-structure",
            );
            output.contains_edges.push(edge);
        }
    }

    // Also create CONTAINS edges between folders (parent folder → child folder)
    let folder_list: Vec<_> = output.all_folders.iter().cloned().collect();
    for folder_path in &folder_list {
        if let Some(parent) = folder_path.parent()
            && let Some(&parent_id) = path_to_id.get(parent)
            && let Some(&child_id) = path_to_id.get(folder_path)
        {
            let edge_id = make_node_id("CONTAINS-FOLDER", folder_path);
            let edge = CodeEdge::new(
                edge_id,
                parent_id,
                child_id,
                EdgeKind::Contains,
                1.0,
                "directory-structure",
            );
            output.contains_edges.push(edge);
        }
    }

    output
}

/// Merge multiple `StructureOutput`s (e.g. from parallel scans of
/// multiple repository roots).
pub fn merge_structures(outputs: Vec<StructureOutput>) -> StructureOutput {
    let mut merged = StructureOutput::default();
    for out in outputs {
        merged.all_paths.extend(out.all_paths);
        merged.all_folders.extend(out.all_folders);
        merged.folder_nodes.extend(out.folder_nodes);
        merged.file_nodes.extend(out.file_nodes);
        merged.contains_edges.extend(out.contains_edges);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::FileInfo;
    use cg_common::Language;

    fn make_file(path: &str, lang: Option<Language>) -> FileInfo {
        FileInfo {
            path: PathBuf::from(path),
            relative_path: PathBuf::from(path),
            size: 0,
            language: lang,
        }
    }

    #[test]
    fn build_simple_structure() {
        let files = vec![
            make_file("src/main.rs", Some(Language::Rust)),
            make_file("src/lib.rs", Some(Language::Rust)),
        ];
        let out = build_structure(&files, Path::new("."));

        assert_eq!(out.file_nodes.len(), 2);
        assert_eq!(out.folder_nodes.len(), 1); // src/
        assert_eq!(out.contains_edges.len(), 2); // src→main.rs, src→lib.rs
    }

    #[test]
    fn build_nested_structure() {
        let files = vec![
            make_file("src/parser/scanner.rs", Some(Language::Rust)),
            make_file("src/parser/structure.rs", Some(Language::Rust)),
            make_file("Cargo.toml", None),
        ];
        let out = build_structure(&files, Path::new("."));

        // Folders: src/, src/parser/
        assert_eq!(out.folder_nodes.len(), 2);
        // Files: 3
        assert_eq!(out.file_nodes.len(), 3);
        // Edges:
        //   src/ → src/parser/
        //   src/parser/ → scanner.rs
        //   src/parser/ → structure.rs
        //   . → Cargo.toml
        assert_eq!(out.contains_edges.len(), 3);
    }

    #[test]
    fn deterministic_ids() {
        let files = vec![make_file("a.rs", Some(Language::Rust))];
        let out1 = build_structure(&files, Path::new("."));
        let out2 = build_structure(&files, Path::new("."));
        assert_eq!(out1.file_nodes[0].id, out2.file_nodes[0].id);
    }

    #[test]
    fn file_language_preserved() {
        let files = vec![make_file("main.rs", Some(Language::Rust))];
        let out = build_structure(&files, Path::new("."));
        assert_eq!(out.file_nodes[0].properties.language, Some(Language::Rust));
    }

    #[test]
    fn merge_outputs() {
        let out1 = build_structure(&[make_file("a.rs", None)], Path::new("."));
        let out2 = build_structure(&[make_file("b.rs", None)], Path::new("."));
        let merged = merge_structures(vec![out1, out2]);
        assert_eq!(merged.file_nodes.len(), 2);
    }
}
