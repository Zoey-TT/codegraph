use cg_common::{CodeEdge, CodeNode, EdgeKind, NodeId, NodeKind, NodeProperties};
use cg_core::process::detect_processes;

use super::{PhaseOutput, PhaseResults, PipelineContext, PipelinePhase};

pub struct ProcessesPhase;

impl PipelinePhase for ProcessesPhase {
    fn name(&self) -> &str {
        "processes"
    }

    fn deps(&self) -> &[&str] {
        &["communities"]
    }

    fn execute(
        &self,
        ctx: &mut PipelineContext,
        _deps: &PhaseResults,
    ) -> anyhow::Result<PhaseOutput> {
        let result = detect_processes(&ctx.graph);

        for (idx, process) in result.processes.iter().enumerate() {
            let process_id = NodeId::new((ctx.graph.node_count() as u64) + 1 + idx as u64);
            let label = format!("Process_{}_{}", process.entry_reason.replace(' ', "_"), idx);
            let process_node = CodeNode::new(
                process_id,
                NodeKind::Process,
                NodeProperties::new(&label, ""),
            );
            ctx.graph.add_node(process_node);

            // EntryPointOf edge
            let entry_edge = CodeEdge::new(
                NodeId::new(process_id.0.wrapping_add(1_000_000)),
                process.entry_id,
                process_id,
                EdgeKind::EntryPointOf,
                process.entry_score,
                "entry point",
            );
            ctx.graph.add_edge(entry_edge);

            // STEP_IN_PROCESS edges
            for (step_idx, step) in process.steps.iter().enumerate() {
                let edge = CodeEdge::new(
                    NodeId::new(process_id.0.wrapping_add(2_000_000 + step_idx as u64)),
                    step.node_id,
                    process_id,
                    EdgeKind::StepInProcess,
                    1.0 - (step.depth as f64 * 0.05), // decay with depth
                    if step.depth == 0 { "entry" } else { "step" },
                );
                ctx.graph.add_edge(edge);
            }
        }

        Ok(Box::new(()))
    }
}
