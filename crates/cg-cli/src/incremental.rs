//! Incremental indexing orchestration.
//!
//! Loads an existing graph, computes git diff + content-hash changes,
//! removes stale nodes, re-parses changed files, and re-runs downstream phases.

use std::path::{Path, PathBuf};

use cg_core::community::CommunityDetector;
use cg_core::incremental::{ContentHashes, FileDelta, compute_file_delta, remove_files_nodes};
use cg_core::{ApplyDelta, GraphDelta};
use cg_graph::InMemoryGraphStore;
use cg_parser::pipeline::parse::parse_files;
use cg_parser::scanner::FileInfo;

/// Result of an incremental index run.
#[derive(Debug)]
#[allow(dead_code)]
pub struct IncrementalResult {
    pub old_nodes: usize,
    pub old_edges: usize,
    pub files_added: usize,
    pub files_modified: usize,
    pub files_deleted: usize,
    pub new_nodes: usize,
    pub new_edges: usize,
    pub duration_secs: f64,
}

/// Run incremental indexing on a repository that already has an index.
///
/// 1. Load old graph from `.codegraph/` JSONL.
/// 2. Compute git diff from `lastCommit` → current HEAD.
/// 3. Remove nodes for modified / deleted / renamed-source files.
/// 4. Re-parse added / modified / renamed-target files.
/// 5. Re-run community detection.
/// 6. Export JSONL and update meta.json.
pub fn run_incremental_index(repo_path: &Path) -> anyhow::Result<IncrementalResult> {
    let start = std::time::Instant::now();
    let codegraph_dir = repo_path.join(".codegraph");

    // --- Load old graph ---
    let store = InMemoryGraphStore::import_jsonl(&codegraph_dir)?;
    let graph = store.knowledge_graph().clone();
    let old_nodes = graph.node_count();
    let old_edges = graph.edge_count();

    // --- Read meta ---
    let meta_path = codegraph_dir.join("meta.json");
    let meta_str = std::fs::read_to_string(&meta_path)?;
    let meta: serde_json::Value = serde_json::from_str(&meta_str)?;
    let old_commit = meta["lastCommit"].as_str().unwrap_or("");
    if old_commit.is_empty() {
        anyhow::bail!("meta.json missing lastCommit; cannot compute delta");
    }

    // --- Current commit ---
    let new_commit = get_git_head(repo_path)?;

    // --- Compute git diff ---
    let delta = if old_commit == new_commit {
        FileDelta::default()
    } else {
        compute_file_delta(repo_path, old_commit, &new_commit)?
    };

    // --- Content hash guard ---
    let hash_path = codegraph_dir.join("hashes.json");
    let old_hashes = ContentHashes::load(&hash_path)?;

    // Merge git diff with hash-changed files (catches changes made outside git,
    // or when git diff is unavailable e.g. shallow clone).
    let all_repo_files = collect_source_files(repo_path)?;
    let hash_changed = old_hashes.changed_files(&all_repo_files);

    // Build unified change set
    let mut to_remove: Vec<PathBuf> = delta
        .files_to_remove()
        .into_iter()
        .map(|p| p.to_path_buf())
        .collect();
    let mut to_parse: Vec<PathBuf> = delta
        .files_to_parse()
        .into_iter()
        .map(|p| p.to_path_buf())
        .collect();

    for path in hash_changed {
        let path = path.to_path_buf();
        if !to_parse.contains(&path) {
            to_parse.push(path.clone());
        }
        if !to_remove.contains(&path) {
            to_remove.push(path);
        }
    }

    // Also remove nodes for files that no longer exist (deleted outside git diff)
    let existing_files: std::collections::HashSet<_> = all_repo_files.iter().cloned().collect();
    for entry in graph.file_index.iter() {
        let file_path = entry.key();
        if !existing_files.contains(file_path.as_path()) && !to_remove.contains(file_path) {
            to_remove.push(file_path.clone());
        }
    }

    // Deduplicate
    to_remove.sort();
    to_remove.dedup();
    to_parse.sort();
    to_parse.dedup();

    // --- Remove stale nodes ---
    for path in &to_remove {
        remove_files_nodes(&graph, &[path.as_path()]);
    }

    // --- Re-parse changed files ---
    let file_infos: Vec<FileInfo> = to_parse
        .iter()
        .filter_map(|p| FileInfo::from_path(p, repo_path).ok())
        .collect();

    let mut parsed_count = 0usize;
    let mut symbol_count = 0usize;
    let mut call_count = 0usize;
    let mut import_count = 0usize;

    if !file_infos.is_empty() {
        let providers = cg_parser::providers::ProviderRegistry::new();
        let (delta, output) = parse_files(&file_infos, repo_path, &providers)?;
        graph.apply_delta(delta);
        parsed_count = output.parsed_count;
        symbol_count = output.symbol_count;
        call_count = output.call_edge_count;
        import_count = output.import_edge_count;
    }

    // --- Re-run community detection ---
    // First remove old Community nodes and MEMBER_OF edges
    let old_community_nodes: Vec<_> = graph
        .nodes_by_kind(cg_common::NodeKind::Community)
        .into_iter()
        .map(|n| n.id)
        .collect();
    for id in old_community_nodes {
        graph.remove_node(&id);
    }

    let detector = CommunityDetector::new();
    let communities = detector.detect(&graph);
    let mut community_delta = GraphDelta::new();
    for community in &communities {
        let community_id =
            cg_common::NodeId::new((graph.node_count() as u64) + 1 + community.id as u64);
        let node = cg_common::CodeNode::new(
            community_id,
            cg_common::NodeKind::Community,
            cg_common::NodeProperties::new(&community.label, ""),
        );
        community_delta.nodes_to_add.push(node);

        for &member_id in &community.members {
            let edge = cg_common::CodeEdge::new(
                cg_common::NodeId::new(
                    (graph.edge_count() as u64) + 1 + community_delta.edges_to_add.len() as u64,
                ),
                member_id,
                community_id,
                cg_common::EdgeKind::MemberOf,
                community.cohesion,
                "leiden",
            );
            community_delta.edges_to_add.push(edge);
        }
    }
    graph.apply_delta(community_delta);

    // --- Save ---
    let mem_store = InMemoryGraphStore::from_knowledge_graph(graph.clone());
    mem_store.export_jsonl(&codegraph_dir)?;

    // Update hashes
    let mut new_hashes = ContentHashes::default();
    for path in &all_repo_files {
        if let Ok(hash) = ContentHashes::hash_file(path) {
            new_hashes.hashes.insert(
                path.strip_prefix(repo_path).unwrap_or(path).to_path_buf(),
                hash,
            );
        }
    }
    new_hashes.save(&hash_path)?;

    // Update meta
    let meta = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "indexedAt": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        "lastCommit": new_commit,
        "stats": {
            "nodes": graph.node_count(),
            "edges": graph.edge_count(),
        },
    });
    std::fs::write(meta_path, serde_json::to_string_pretty(&meta)?)?;

    let duration_secs = start.elapsed().as_secs_f64();

    println!("📊 Incremental update:");
    println!("  Old: {} nodes, {} edges", old_nodes, old_edges);
    println!("  Removed: {} files", to_remove.len());
    println!(
        "  Re-parsed: {} files ({} symbols, {} calls, {} imports)",
        parsed_count, symbol_count, call_count, import_count
    );
    println!("  Communities: {}", communities.len());
    println!(
        "  New: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );
    println!("  Time: {:.2}s", duration_secs);

    Ok(IncrementalResult {
        old_nodes,
        old_edges,
        files_added: delta.added.len(),
        files_modified: delta.modified.len(),
        files_deleted: delta.deleted.len(),
        new_nodes: graph.node_count(),
        new_edges: graph.edge_count(),
        duration_secs,
    })
}

/// Collect all source files in the repository.
fn collect_source_files(repo_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let options = cg_parser::scanner::ScanOptions::default();
    let result = cg_parser::scanner::scan_directory(repo_path, &options)?;
    Ok(result.files.into_iter().map(|f| f.path).collect())
}

fn get_git_head(path: &Path) -> anyhow::Result<String> {
    let repo = git2::Repository::discover(path)?;
    let head = repo.head()?;
    let oid = head
        .target()
        .ok_or_else(|| anyhow::anyhow!("detached HEAD"))?;
    Ok(oid.to_string())
}
