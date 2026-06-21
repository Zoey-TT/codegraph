use super::{PhaseOutput, PhaseResults, PipelineContext, PipelinePhase, scan};
use crate::extractors::ExtractedCall;
use crate::parser::pool::ParserPool;
use crate::resolution::{SymbolTable, build_import_edges, resolve_call};
use crate::structure::make_node_id;
use cg_common::{CodeEdge, CodeNode, EdgeKind, NodeProperties};
use cg_core::{ApplyDelta, GraphDelta};
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

pub struct ParsePhase;

impl PipelinePhase for ParsePhase {
    fn name(&self) -> &str {
        "parse"
    }

    fn deps(&self) -> &[&str] {
        &["scan", "structure"]
    }

    fn execute(
        &self,
        ctx: &mut PipelineContext,
        deps: &PhaseResults,
    ) -> anyhow::Result<PhaseOutput> {
        let scan_result = scan::get_scan_result(deps)
            .ok_or_else(|| anyhow::anyhow!("parse phase missing scan dependency"))?;

        let (delta, output) = parse_files(&scan_result.files, &ctx.repo_path, &ctx.providers)?;
        ctx.graph.apply_delta(delta);

        Ok(Box::new(output))
    }
}

/// Parse a subset of files and produce a `GraphDelta` + statistics.
///
/// This is the reusable core of `ParsePhase`, exposed so that incremental
/// indexing can re-parse only changed files.
pub fn parse_files(
    files: &[crate::scanner::FileInfo],
    repo_path: &std::path::Path,
    providers: &crate::providers::ProviderRegistry,
) -> anyhow::Result<(GraphDelta, ParsePhaseOutput)> {
    let pool = ParserPool::new(rayon::current_num_threads())?;
    let parsed = pool.parse_batch(files);

    let mut symbol_count = 0usize;
    let mut call_edge_count = 0usize;
    let mut import_edge_count = 0usize;
    let mut calls: CallsByFile = Vec::new();
    let mut delta = GraphDelta::new();

    for parsed_file in &parsed {
        let file_id = make_node_id(
            "File",
            parsed_file
                .file_path
                .strip_prefix(repo_path)
                .unwrap_or(&parsed_file.file_path),
        );

        if let Some(provider) = providers.get(parsed_file.language) {
            // --- Symbols ---
            let mut table = SymbolTable::new();
            if let Some(extractor) = provider.symbol_extractor() {
                let symbols = extractor.extract(parsed_file);
                for sym in &symbols {
                    let mut props = NodeProperties::new(&sym.name, &parsed_file.file_path);
                    props.language = Some(parsed_file.language);
                    props.start_line = Some(sym.range.start_point.row as u32);
                    props.end_line = Some(sym.range.end_point.row as u32);
                    let node = CodeNode::new(sym.id, sym.kind, props);
                    delta.nodes_to_add.push(node);

                    // CONTAINS edge from file to symbol
                    let edge_id = make_node_id("CONTAINS", &parsed_file.file_path.join(&sym.name));
                    let edge = CodeEdge::new(
                        edge_id,
                        file_id,
                        sym.id,
                        EdgeKind::Contains,
                        1.0,
                        "parent file",
                    );
                    delta.edges_to_add.push(edge);
                    table.add_local(&sym.name, sym.id);
                    symbol_count += 1;
                }
            }

            // --- Imports ---
            if let Some(extractor) = provider.import_extractor() {
                let imports = extractor.extract(parsed_file);
                for imp in &imports {
                    for name in &imp.names {
                        table.add_import(
                            &name.local_name,
                            name.original_name
                                .clone()
                                .unwrap_or_else(|| name.local_name.clone()),
                            imp.source.clone(),
                        );
                    }
                }
                let import_edges = build_import_edges(file_id, &imports);
                import_edge_count += import_edges.len();
                delta.edges_to_add.extend(import_edges);
            }

            // --- Heritage (EXTENDS / IMPLEMENTS / INHERITS) ---
            if let Some(extractor) = provider.heritage_extractor() {
                let heritages = extractor.extract(parsed_file);
                for h in &heritages {
                    let edge_kind = match h.kind {
                        crate::extractors::HeritageKind::Extends => EdgeKind::Extends,
                        crate::extractors::HeritageKind::Implements => EdgeKind::Implements,
                        crate::extractors::HeritageKind::Includes => EdgeKind::Inherits,
                    };
                    // Create a deterministic NodeId for the parent reference.
                    // Real resolution will happen in the MRO / CrossFile phases.
                    let mut hasher = FxHasher::default();
                    "heritage".hash(&mut hasher);
                    h.parent_name.hash(&mut hasher);
                    let parent_id = cg_common::NodeId::new(hasher.finish());

                    delta.edges_to_add.push(CodeEdge::new(
                        make_node_id(
                            &format!("HERITAGE_{:?}", edge_kind),
                            &parsed_file.file_path.join(&h.parent_name),
                        ),
                        h.child_id,
                        parent_id,
                        edge_kind,
                        0.80,
                        format!("{} → {}", h.child_id.0, h.parent_name),
                    ));
                }
            }

            // --- Calls (Tier 1 resolution only) ---
            let file_calls = if let Some(extractor) = provider.call_extractor() {
                extractor.extract(parsed_file)
            } else {
                Vec::new()
            };
            for call in &file_calls {
                // Try Tier 1: same-file symbol lookup
                if let Some(edge) = resolve_call(call.caller_id, &call.callee_name, &table) {
                    delta.edges_to_add.push(edge);
                    call_edge_count += 1;
                }
            }
            if !file_calls.is_empty() {
                calls.push((parsed_file.file_path.clone(), file_calls));
            }
        }
    }

    let output = ParsePhaseOutput {
        parsed_count: parsed.len(),
        symbol_count,
        call_edge_count,
        import_edge_count,
        calls,
    };

    Ok((delta, output))
}

/// Raw calls extracted during the parse phase, keyed by file path.
/// The resolve phase uses this data to perform full cross-file call resolution.
pub type CallsByFile = Vec<(PathBuf, Vec<ExtractedCall>)>;

#[derive(Debug)]
pub struct ParsePhaseOutput {
    pub parsed_count: usize,
    pub symbol_count: usize,
    pub call_edge_count: usize,
    pub import_edge_count: usize,
    /// All extracted calls per file, including those that could not be
    /// resolved at Tier 1 (same-file) during parsing.
    pub calls: CallsByFile,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_phase_deps() {
        let phase = ParsePhase;
        assert_eq!(phase.deps(), &["scan", "structure"]);
    }
}
