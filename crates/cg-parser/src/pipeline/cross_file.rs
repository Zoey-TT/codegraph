use cg_core::ApplyDelta;

use super::{PhaseOutput, PhaseResults, PipelineContext, PipelinePhase};
use crate::resolution::cross_file::{CrossFileIndex, infer_variable_types};

pub struct CrossFilePhase;

impl PipelinePhase for CrossFilePhase {
    fn name(&self) -> &str {
        "crossFile"
    }

    fn deps(&self) -> &[&str] {
        &["parse"]
    }

    fn execute(
        &self,
        ctx: &mut PipelineContext,
        _deps: &PhaseResults,
    ) -> anyhow::Result<PhaseOutput> {
        let index = CrossFileIndex::build(&ctx.graph);

        // Apply type inferences to graph
        let delta = infer_variable_types(&ctx.graph);
        if !delta.is_empty() {
            ctx.graph.apply_delta(delta);
        }

        Ok(Box::new(index))
    }
}
