use super::{PhaseOutput, PhaseResults, PipelineContext, PipelinePhase, scan};
use crate::structure::build_structure;
use cg_core::{ApplyDelta, GraphDelta};

pub struct StructurePhase;

impl PipelinePhase for StructurePhase {
    fn name(&self) -> &str {
        "structure"
    }

    fn deps(&self) -> &[&str] {
        &["scan"]
    }

    fn execute(
        &self,
        ctx: &mut PipelineContext,
        deps: &PhaseResults,
    ) -> anyhow::Result<PhaseOutput> {
        let scan_result = scan::get_scan_result(deps)
            .ok_or_else(|| anyhow::anyhow!("structure phase missing scan dependency"))?;

        let structure = build_structure(&scan_result.files, &ctx.repo_path);

        // Apply to knowledge graph
        let mut delta = GraphDelta::new();
        delta.nodes_to_add.extend(structure.folder_nodes.clone());
        delta.nodes_to_add.extend(structure.file_nodes.clone());
        delta.edges_to_add.extend(structure.contains_edges.clone());
        ctx.graph.apply_delta(delta);

        Ok(Box::new(structure))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::downcast_output;
    use crate::scanner::FileInfo;
    use cg_common::Language;
    use std::path::PathBuf;

    #[test]
    fn structure_phase_smoke() {
        let files = vec![
            FileInfo {
                path: PathBuf::from("/tmp/src/main.rs"),
                relative_path: PathBuf::from("src/main.rs"),
                size: 100,
                language: Some(Language::Rust),
            },
            FileInfo {
                path: PathBuf::from("/tmp/src/lib.rs"),
                relative_path: PathBuf::from("src/lib.rs"),
                size: 50,
                language: Some(Language::Rust),
            },
        ];

        let mut ctx = PipelineContext::new(".");
        let mut deps = PhaseResults::new();
        deps.insert(
            "scan",
            Box::new(crate::scanner::ScanResult {
                files,
                all_paths: std::collections::HashSet::new(),
                skipped_large: 0,
                skipped_ignored: 0,
                total_seen: 2,
            }),
        );

        let phase = StructurePhase;
        let output = phase.execute(&mut ctx, &deps).unwrap();
        let structure = downcast_output::<crate::structure::StructureOutput>(&output).unwrap();

        assert!(!structure.folder_nodes.is_empty()); // "src" folder
        assert_eq!(structure.file_nodes.len(), 2);
        assert!(!structure.contains_edges.is_empty());
    }
}
