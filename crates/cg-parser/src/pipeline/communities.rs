use cg_common::{CodeEdge, CodeNode, EdgeKind, NodeId, NodeKind, NodeProperties};
use cg_core::community::CommunityDetector;

use super::{PhaseOutput, PhaseResults, PipelineContext, PipelinePhase};

pub struct CommunitiesPhase;

impl PipelinePhase for CommunitiesPhase {
    fn name(&self) -> &str {
        "communities"
    }

    fn deps(&self) -> &[&str] {
        &["mro", "resolve", "parse", "structure"]
    }

    fn execute(
        &self,
        ctx: &mut PipelineContext,
        _deps: &PhaseResults,
    ) -> anyhow::Result<PhaseOutput> {
        let detector = CommunityDetector::new();
        let communities = detector.detect(&ctx.graph);

        let mut community_count = 0usize;
        let mut edge_count = 0usize;

        for community in &communities {
            let community_id =
                NodeId::new((ctx.graph.node_count() as u64) + 1 + community.id as u64);
            let community_node = CodeNode::new(
                community_id,
                NodeKind::Community,
                NodeProperties::new(&community.label, ""),
            );
            ctx.graph.add_node(community_node);
            community_count += 1;

            for &member_id in &community.members {
                let edge = CodeEdge::new(
                    NodeId::new((ctx.graph.edge_count() as u64) + 1 + edge_count as u64),
                    member_id,
                    community_id,
                    EdgeKind::MemberOf,
                    community.cohesion,
                    "leiden",
                );
                ctx.graph.add_edge(edge);
                edge_count += 1;
            }
        }

        Ok(Box::new(CommunitiesPhaseOutput {
            community_count,
            member_edge_count: edge_count,
        }))
    }
}

#[derive(Debug)]
pub struct CommunitiesPhaseOutput {
    pub community_count: usize,
    pub member_edge_count: usize,
}
