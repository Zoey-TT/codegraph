//! CodeGraph CLI entry point.
//!
//! ```bash
//! codegraph analyze [path] [--force] [--verbose]
//! codegraph setup
//! codegraph mcp
//! codegraph serve [--host] [--port]
//! codegraph list
//! codegraph status
//! codegraph clean [--all] [--force]
//! codegraph query <query> [--repo]
//! codegraph context <name> [--repo]
//! codegraph impact <target> [--direction]
//! ```

use cg_cli::{incremental, registry};

use cg_graph::GraphStore;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "codegraph")]
#[command(about = "CodeGraph — knowledge graph for codebases")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Analyze a repository and build its knowledge graph.
    Analyze {
        /// Repository path (defaults to current directory).
        path: Option<PathBuf>,
        /// Force full re-index even if index is up-to-date.
        #[arg(long)]
        force: bool,
        /// Enable verbose output.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Configure MCP integration.
    Setup,
    /// Start MCP server (stdio).
    Mcp,
    /// Start HTTP API server.
    Serve {
        /// Bind host.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Bind port.
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
    /// List all indexed repositories.
    List,
    /// Show current repository index status.
    Status,
    /// Clean index data.
    Clean {
        /// Clean all repositories.
        #[arg(long)]
        all: bool,
        /// Skip confirmation.
        #[arg(long)]
        force: bool,
    },
    /// Hybrid search query.
    Query {
        /// Search query string.
        query: String,
        /// Restrict to a specific repository.
        #[arg(long)]
        repo: Option<String>,
    },
    /// Show 360° context for a symbol.
    Context {
        /// Symbol name.
        name: String,
        /// Repository scope.
        #[arg(long)]
        repo: Option<String>,
    },
    /// Impact analysis (blast radius).
    Impact {
        /// Target symbol.
        target: String,
        /// Direction: upstream, downstream, or both.
        #[arg(long, default_value = "both")]
        direction: Direction,
    },
    /// Detect functional communities in the indexed codebase.
    Communities {
        /// Repository path.
        #[arg(long)]
        repo: Option<String>,
        /// Resolution parameter γ (lower = fewer communities).
        #[arg(long, default_value_t = 0.5)]
        resolution: f64,
        /// Minimum community size.
        #[arg(long, default_value_t = 2)]
        min_size: usize,
    },
}

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
enum Direction {
    Upstream,
    Downstream,
    #[default]
    Both,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Command::Analyze {
            path,
            force,
            verbose,
        } => {
            let path = path.unwrap_or_else(|| std::env::current_dir().unwrap());
            println!("Analyzing: {}", path.display());
            if verbose {
                println!("  --verbose");
            }

            // --- Incremental index check ---
            let current_commit = get_git_head(&path).unwrap_or_default();
            let meta_path = path.join(".codegraph").join("meta.json");
            let has_existing_index = meta_path.exists();

            let needs_reindex = if force {
                true
            } else if let Ok(meta_str) = std::fs::read_to_string(&meta_path) {
                if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&meta_str) {
                    let last_commit = meta
                        .get("lastCommit")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !current_commit.is_empty() && last_commit == current_commit {
                        println!(
                            "\n✅ Index is up-to-date (commit {}). Use --force to re-index.",
                            &current_commit[..7.min(current_commit.len())]
                        );
                        false
                    } else {
                        true
                    }
                } else {
                    true
                }
            } else {
                true
            };

            if !needs_reindex {
                return Ok(());
            }

            // --- Try incremental update if we have an existing index and not --force ---
            if has_existing_index && !force {
                println!("\n🔄 Attempting incremental update...");
                match incremental::run_incremental_index(&path) {
                    Ok(result) => {
                        println!(
                            "\n💾 Incremental index updated in {:.2}s",
                            result.duration_secs
                        );
                        return Ok(());
                    }
                    Err(e) => {
                        println!(
                            "⚠️  Incremental update failed ({}), falling back to full re-index...",
                            e
                        );
                    }
                }
            }

            // --- Full re-index ---
            let spinner = ProgressBar::new_spinner();
            spinner.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} [{elapsed_precise}] {msg}")
                    .unwrap(),
            );

            let start = Instant::now();

            let runner = cg_parser::pipeline::build_full_pipeline();
            let mut ctx = cg_parser::pipeline::PipelineContext::new(&path);

            spinner.set_message("Running pipeline...");
            let results = match runner.run(&mut ctx) {
                Ok(r) => r,
                Err(e) => {
                    spinner.finish_with_message(format!("Pipeline failed: {}", e));
                    anyhow::bail!(e);
                }
            };

            spinner.finish_with_message(format!(
                "Pipeline completed in {:.2}s",
                start.elapsed().as_secs_f64()
            ));

            // Print stats
            let graph = &ctx.graph;
            println!("\n📊 Results:");
            println!("  Nodes: {}", graph.node_count());
            println!("  Edges: {}", graph.edge_count());

            if let Some(parse_output) = results.get("parse")
                && let Some(output) =
                    parse_output.downcast_ref::<cg_parser::pipeline::parse::ParsePhaseOutput>()
            {
                println!("  Parsed files: {}", output.parsed_count);
                println!("  Symbols: {}", output.symbol_count);
                println!("  Call edges: {}", output.call_edge_count);
                println!("  Import edges: {}", output.import_edge_count);
            }

            // Write meta.json
            let codegraph_dir = path.join(".codegraph");
            std::fs::create_dir_all(&codegraph_dir)?;
            let meta = serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "indexedAt": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
                "lastCommit": current_commit,
                "stats": {
                    "nodes": graph.node_count(),
                    "edges": graph.edge_count(),
                },
            });
            std::fs::write(
                codegraph_dir.join("meta.json"),
                serde_json::to_string_pretty(&meta)?,
            )?;

            // Export JSONL for fast query reload
            let mem_store = cg_graph::InMemoryGraphStore::from_knowledge_graph(ctx.graph.clone());
            mem_store.export_jsonl(&codegraph_dir)?;

            // Initial content hashes
            let mut hashes = cg_core::incremental::ContentHashes::default();
            if let Ok(files) = cg_parser::scanner::scan_directory(
                &path,
                &cg_parser::scanner::ScanOptions::default(),
            ) {
                for file in &files.files {
                    if let Ok(hash) = cg_core::incremental::ContentHashes::hash_file(&file.path) {
                        hashes.hashes.insert(file.relative_path.clone(), hash);
                    }
                }
            }
            let _ = hashes.save(&codegraph_dir.join("hashes.json"));

            // Register in global registry
            if let Err(e) =
                registry::register_repo(&path, Some(graph.node_count()), Some(graph.edge_count()))
            {
                eprintln!("⚠️  Failed to update registry: {}", e);
            }

            println!("\n💾 Index written to {}", codegraph_dir.display());
        }
        Command::Setup => {
            println!("🔧 Setting up MCP integration...\n");
            match cg_mcp::setup_all_clients() {
                Ok(results) => {
                    for res in &results {
                        let action_str = match res.action {
                            cg_mcp::SetupAction::Added => "✅ Added",
                            cg_mcp::SetupAction::Updated => "🔄 Updated",
                            cg_mcp::SetupAction::Unchanged => "⏭️  Already configured",
                        };
                        println!(
                            "{} {} → {}",
                            action_str,
                            res.client.display_name(),
                            res.config_path.display()
                        );
                        if res.client.hot_reload() {
                            println!("   (hot-reload: no restart needed)");
                        } else {
                            println!(
                                "   ⚠️  Please restart {} for changes to take effect",
                                res.client.display_name()
                            );
                        }
                    }
                    println!("\n🚀 CodeGraph MCP server is ready to use!");
                }
                Err(e) => {
                    eprintln!("❌ No MCP clients detected: {}", e);
                    eprintln!("\n💡 Manual configuration:");
                    match cg_mcp::generate_manual_config() {
                        Ok(json) => println!("{}", json),
                        Err(e2) => eprintln!("Failed to generate config: {}", e2),
                    }
                }
            }
        }
        Command::Mcp => {
            let path = std::env::current_dir()?;
            tracing::info!("Starting MCP server for {}", path.display());
            cg_mcp::run_stdio_server(&path).await?;
        }
        Command::Serve { host, port } => {
            let path = std::env::current_dir()?;
            println!("Starting HTTP server on http://{}:{}", host, port);
            println!("Serving repo: {}", path.display());
            cg_server::serve(&host, port, &path).await?;
        }
        Command::List => match registry::load_registry() {
            Ok(entries) if !entries.is_empty() => {
                println!("Indexed repositories ({}):", entries.len());
                for entry in entries {
                    println!("  📁 {} ({})", entry.name, entry.path.display());
                    if let (Some(n), Some(e)) = (entry.node_count, entry.edge_count) {
                        println!("     Nodes: {}, Edges: {}", n, e);
                    }
                }
            }
            _ => {
                println!("No indexed repositories found.");
                println!("💡 Run `codegraph analyze <path>` to index a repository.");
            }
        },
        Command::Status => {
            let path = std::env::current_dir()?;
            let meta_path = path.join(".codegraph").join("meta.json");

            if let Ok(meta_str) = std::fs::read_to_string(&meta_path) {
                if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&meta_str) {
                    let nodes = meta["stats"]["nodes"].as_u64().unwrap_or(0);
                    let edges = meta["stats"]["edges"].as_u64().unwrap_or(0);
                    let last_commit = meta["lastCommit"].as_str().unwrap_or("unknown");
                    let indexed_at = meta["indexedAt"].as_i64().unwrap_or(0);

                    println!("📁 Repository: {}", path.display());
                    println!("   Nodes: {}", nodes);
                    println!("   Edges: {}", edges);
                    println!(
                        "   Last commit: {}",
                        if last_commit.len() >= 7 {
                            &last_commit[..7]
                        } else {
                            last_commit
                        }
                    );
                    println!("   Indexed at: {} (Unix timestamp)", indexed_at);

                    if let Ok(current) = get_git_head(&path) {
                        if current == last_commit {
                            println!("   Status: ✅ up-to-date");
                        } else {
                            println!(
                                "   Status: ⚠️  stale (current: {})",
                                if current.len() >= 7 {
                                    &current[..7]
                                } else {
                                    &current
                                }
                            );
                        }
                    }
                } else {
                    println!("No valid index found. Run `codegraph analyze` to build.");
                }
            } else {
                println!("No index found. Run `codegraph analyze` to build.");
            }
        }
        Command::Clean { all, force } => {
            if all {
                let entries = registry::load_registry()?;
                if entries.is_empty() {
                    println!("No indexed repositories to clean.");
                    return Ok(());
                }

                if !force {
                    println!(
                        "This will delete indexes for {} repositories. Use --force to confirm.",
                        entries.len()
                    );
                    for entry in &entries {
                        println!("  - {} ({})", entry.name, entry.path.display());
                    }
                    return Ok(());
                }

                let mut cleaned = 0;
                let mut failed = 0;
                for entry in &entries {
                    let codegraph_dir = entry.path.join(".codegraph");
                    if codegraph_dir.exists() {
                        match std::fs::remove_dir_all(&codegraph_dir) {
                            Ok(_) => {
                                println!("Cleaned {}", codegraph_dir.display());
                                cleaned += 1;
                            }
                            Err(e) => {
                                eprintln!("Failed to clean {}: {}", codegraph_dir.display(), e);
                                failed += 1;
                            }
                        }
                    }
                    if let Err(e) = registry::unregister_repo(&entry.path) {
                        eprintln!("Failed to unregister {}: {}", entry.path.display(), e);
                    }
                }
                println!("Cleaned {} repositories ({} failed).", cleaned, failed);
                return Ok(());
            }

            let path = std::env::current_dir()?;
            let codegraph_dir = path.join(".codegraph");
            if !codegraph_dir.exists() {
                println!("No index to clean.");
                return Ok(());
            }
            if !force {
                println!(
                    "This will delete {}. Use --force to confirm.",
                    codegraph_dir.display()
                );
                return Ok(());
            }
            std::fs::remove_dir_all(&codegraph_dir)?;
            if let Err(e) = registry::unregister_repo(&path) {
                eprintln!("Failed to update registry: {}", e);
            }
            println!("Cleaned {}", codegraph_dir.display());
        }
        Command::Query { query, repo } => {
            let path = repo
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap());
            let codegraph_dir = path.join(".codegraph");
            let store = cg_graph::InMemoryGraphStore::import_jsonl(&codegraph_dir)?;
            let searcher = cg_search::MemorySearcher::new(&store);

            let hits = searcher.search_name(&query, None)?;
            if hits.is_empty() {
                println!("No results for '{}'", query);
            } else {
                println!("Results for '{}':", query);
                for (i, hit) in hits.iter().take(20).enumerate() {
                    println!(
                        "  {}. {} ({:?}) — {} [score: {:.2}]",
                        i + 1,
                        hit.node.properties.name,
                        hit.node.kind,
                        hit.node.properties.file_path.display(),
                        hit.score,
                    );
                }
                if hits.len() > 20 {
                    println!("  ... and {} more", hits.len() - 20);
                }
            }
        }
        Command::Context { name, repo } => {
            let path = repo
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap());
            let codegraph_dir = path.join(".codegraph");
            let store = cg_graph::InMemoryGraphStore::import_jsonl(&codegraph_dir)?;
            let searcher = cg_search::MemorySearcher::new(&store);

            let hits = searcher.search_name(&name, None)?;
            if let Some(hit) = hits.first() {
                let ctx = searcher.context(hit.node.id.0)?;
                println!(
                    "🔍 Context for {} ({:?})",
                    ctx.node.properties.name, ctx.node.kind
                );
                println!("   File: {}", ctx.node.properties.file_path.display());
                if let Some(lang) = ctx.node.properties.language {
                    println!("   Language: {:?}", lang);
                }
                if !ctx.callers.is_empty() {
                    println!("   Called by: {} symbols", ctx.callers.len());
                }
                if !ctx.calls.is_empty() {
                    println!("   Calls: {} symbols", ctx.calls.len());
                }
                if !ctx.members.is_empty() {
                    println!("   Members: {} symbols", ctx.members.len());
                }
            } else {
                println!("Symbol '{}' not found.", name);
            }
        }
        Command::Impact { target, direction } => {
            let path = std::env::current_dir()?;
            let codegraph_dir = path.join(".codegraph");
            let store = cg_graph::InMemoryGraphStore::import_jsonl(&codegraph_dir)?;
            let searcher = cg_search::MemorySearcher::new(&store);

            let hits = searcher.search_name(&target, None)?;
            if hits.is_empty() {
                println!("Symbol '{}' not found.", target);
                return Ok(());
            }

            let start_id = hits[0].node.id.0;
            let mut visited = std::collections::HashSet::new();
            let mut results = Vec::new();

            match direction {
                Direction::Upstream => {
                    traverse_callers(&searcher, start_id, 5, &mut results, &mut visited);
                }
                Direction::Downstream => {
                    traverse_callees(&searcher, start_id, 5, &mut results, &mut visited);
                }
                Direction::Both => {
                    traverse_callers(&searcher, start_id, 5, &mut results, &mut visited);
                    visited.clear();
                    traverse_callees(&searcher, start_id, 5, &mut results, &mut visited);
                }
            }

            println!(
                "🔍 Impact analysis for '{}' ({}):",
                target,
                direction_string(direction)
            );
            println!("  {} affected symbols found:", results.len());
            for (depth, id) in &results {
                if let Ok(Some(node)) = store.get_node(&id.to_string()) {
                    println!(
                        "    [d={}] {} ({:?}) — {}",
                        depth,
                        node.properties.name,
                        node.kind,
                        node.properties.file_path.display()
                    );
                }
            }
        }
        Command::Communities {
            repo,
            resolution,
            min_size,
        } => {
            let path = repo
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap());
            let codegraph_dir = path.join(".codegraph");
            let store = cg_graph::InMemoryGraphStore::import_jsonl(&codegraph_dir)?;
            let kg = store.knowledge_graph();

            let detector = cg_core::community::CommunityDetector {
                resolution,
                min_size,
                ..Default::default()
            };
            let communities = detector.detect(kg);

            if communities.is_empty() {
                println!("No communities found. Try lowering --resolution or --min-size.");
            } else {
                println!("Detected {} communities:\n", communities.len());
                for c in &communities {
                    println!(
                        "  📦 {} ({} members, cohesion: {:.2})",
                        c.label,
                        c.members.len(),
                        c.cohesion
                    );
                    // Show top 5 member names
                    let mut names: Vec<String> = c
                        .members
                        .iter()
                        .filter_map(|id| kg.nodes.get(id).map(|n| n.properties.name.clone()))
                        .collect();
                    names.sort();
                    names.truncate(5);
                    if !names.is_empty() {
                        println!("     → {}", names.join(", "));
                    }
                    if c.members.len() > 5 {
                        println!("     ... and {} more", c.members.len() - 5);
                    }
                }
            }
        }
    }

    Ok(())
}

fn get_git_head(path: &std::path::Path) -> anyhow::Result<String> {
    let repo = git2::Repository::discover(path)?;
    let head = repo.head()?;
    let oid = head
        .target()
        .ok_or_else(|| anyhow::anyhow!("detached HEAD"))?;
    Ok(oid.to_string())
}

fn direction_string(d: Direction) -> &'static str {
    match d {
        Direction::Upstream => "upstream",
        Direction::Downstream => "downstream",
        Direction::Both => "both",
    }
}

fn traverse_callers(
    searcher: &cg_search::MemorySearcher,
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
            out.push((6 - max_depth, caller));
            traverse_callers(searcher, caller, max_depth - 1, out, visited);
        }
    }
}

fn traverse_callees(
    searcher: &cg_search::MemorySearcher,
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
            out.push((6 - max_depth, callee));
            traverse_callees(searcher, callee, max_depth - 1, out, visited);
        }
    }
}
