use cg_common::MroStrategy;
use cg_core::mro::{compute_mro, emit_override_edges};

use super::{PhaseOutput, PhaseResults, PipelineContext, PipelinePhase};

pub struct MroPhase;

impl PipelinePhase for MroPhase {
    fn name(&self) -> &str {
        "mro"
    }

    fn deps(&self) -> &[&str] {
        &["crossFile"]
    }

    fn execute(
        &self,
        ctx: &mut PipelineContext,
        _deps: &PhaseResults,
    ) -> anyhow::Result<PhaseOutput> {
        let cache = compute_mro(&ctx.graph, MroStrategy::C3);

        // Write MRO results into node extras
        for (&node_id, mro) in &cache.mro {
            if let Some(mut node) = ctx.graph.get_node(&node_id) {
                let ids: Vec<u64> = mro.iter().map(|n| n.0).collect();
                node.properties
                    .extras
                    .insert("mro".to_string(), serde_json::json!(ids));
                // Remove old and add updated (graph does not support in-place mutation)
                ctx.graph.remove_node(&node_id);
                ctx.graph.add_node(node);
            }
        }

        // Emit METHOD_OVERRIDES / METHOD_IMPLEMENTS edges
        let override_edges = emit_override_edges(&ctx.graph, &cache);
        for edge in override_edges {
            ctx.graph.add_edge(edge);
        }

        Ok(Box::new(cache))
    }
}
