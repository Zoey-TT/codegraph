//! Call resolution phase.
//!
//! Re-visits all CALLS edges produced during the `parse` phase and
//! re-resolves them using the cross-file index and MRO cache.

use cg_common::EdgeKind;
use cg_core::mro::MroCache;
use cg_core::{ApplyDelta, GraphDelta};

use crate::pipeline::parse::ParsePhaseOutput;
use crate::resolution::cross_file::CrossFileIndex;
use crate::resolution::resolve::{ResolveContext, resolve_call_full, resolved_call_to_edge};

use super::{PhaseOutput, PhaseResults, PipelineContext, PipelinePhase, downcast_output};

pub struct ResolvePhase;

impl PipelinePhase for ResolvePhase {
    fn name(&self) -> &str {
        "resolve"
    }

    fn deps(&self) -> &[&str] {
        &["crossFile", "mro", "parse"]
    }

    fn execute(
        &self,
        ctx: &mut PipelineContext,
        deps: &PhaseResults,
    ) -> anyhow::Result<PhaseOutput> {
        // Retrieve CrossFileIndex from crossFile phase
        let cross_file = deps
            .get("crossFile")
            .and_then(|o| downcast_output::<CrossFileIndex>(o))
            .ok_or_else(|| anyhow::anyhow!("resolve phase missing crossFile dependency"))?;

        // Retrieve MroCache from mro phase
        let mro = deps
            .get("mro")
            .and_then(|o| downcast_output::<MroCache>(o))
            .ok_or_else(|| anyhow::anyhow!("resolve phase missing mro dependency"))?;

        // Retrieve ParsePhaseOutput from parse phase
        let parse_output = deps
            .get("parse")
            .and_then(|o| downcast_output::<ParsePhaseOutput>(o))
            .ok_or_else(|| anyhow::anyhow!("resolve phase missing parse dependency"))?;

        let resolve_ctx = ResolveContext {
            graph: &ctx.graph,
            cross_file,
            mro,
        };

        let mut resolved_count = 0usize;
        let mut unresolved_count = 0usize;
        let mut delta = GraphDelta::new();

        // For each file, remove all tentative CALLS edges from callers in that file
        // and re-resolve every ExtractedCall using the full three-tier system.
        for (_file_path, calls) in &parse_output.calls {
            // Collect all unique caller ids in this file
            let caller_ids: Vec<_> = calls
                .iter()
                .map(|c| c.caller_id)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();

            // Remove old CALLS edges from these callers
            for caller_id in &caller_ids {
                ctx.graph
                    .remove_outgoing_edges_by_kind(caller_id, EdgeKind::Calls);
            }

            // Re-resolve every call
            for call in calls {
                let resolved = resolve_call_full(&resolve_ctx, call.caller_id, call);

                if resolved.is_empty() {
                    unresolved_count += 1;
                    continue;
                }

                // Add resolved edges (deduplicated by target, keeping highest confidence)
                let mut best_by_target: std::collections::HashMap<
                    cg_common::NodeId,
                    crate::resolution::resolve::ResolvedCall,
                > = std::collections::HashMap::new();
                for r in resolved {
                    best_by_target
                        .entry(r.target_id)
                        .and_modify(|e| {
                            if r.confidence > e.confidence {
                                *e = r.clone();
                            }
                        })
                        .or_insert(r);
                }

                for (_, best) in best_by_target {
                    delta
                        .edges_to_add
                        .push(resolved_call_to_edge(call.caller_id, &best));
                    resolved_count += 1;
                }
            }
        }

        // Apply the delta
        ctx.graph.apply_delta(delta);

        let output = ResolvePhaseOutput {
            resolved_count,
            unresolved_count,
        };

        Ok(Box::new(output))
    }
}

#[derive(Debug)]
pub struct ResolvePhaseOutput {
    pub resolved_count: usize,
    pub unresolved_count: usize,
}
