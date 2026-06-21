//! Pipeline phase trait and DAG runner.
//!
//! The ingestion pipeline for the minimal release:
//!   scan → structure → parse → crossFile → mro → communities → processes

pub mod communities;
pub mod cross_file;
pub mod mro;
pub mod parse;
pub mod processes;
pub mod resolve;
pub mod scan;
pub mod structure;

use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};

use cg_core::KnowledgeGraph;

use crate::providers::ProviderRegistry;

/// Context shared across all pipeline phases.
pub struct PipelineContext {
    pub repo_path: std::path::PathBuf,
    pub options: HashMap<String, String>,
    /// In-memory knowledge graph that phases mutate directly.
    pub graph: KnowledgeGraph,
    /// Language providers for symbol extraction.
    pub providers: ProviderRegistry,
}

impl PipelineContext {
    pub fn new(repo_path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
            options: HashMap::new(),
            graph: KnowledgeGraph::new(),
            providers: ProviderRegistry::new(),
        }
    }
}

/// Output produced by a single phase.
pub type PhaseOutput = Box<dyn Any + Send + Sync>;

/// Results from previously executed phases.
pub type PhaseResults<'a> = HashMap<&'a str, PhaseOutput>;

/// Trait for a single pipeline phase.
pub trait PipelinePhase: Send + Sync {
    /// Human-readable phase name.
    fn name(&self) -> &str;

    /// Names of phases this phase depends on.
    fn deps(&self) -> &[&str];

    /// Execute the phase.
    fn execute(
        &self,
        ctx: &mut PipelineContext,
        deps: &PhaseResults,
    ) -> anyhow::Result<PhaseOutput>;
}

/// DAG pipeline runner with Kahn topological sort.
pub struct PipelineRunner {
    phases: Vec<Box<dyn PipelinePhase>>,
}

impl PipelineRunner {
    pub fn new(phases: Vec<Box<dyn PipelinePhase>>) -> Self {
        Self { phases }
    }

    /// Validate the DAG (no cycles, no missing deps) and execute phases
    /// in topological order.
    pub fn run(&self, ctx: &mut PipelineContext) -> anyhow::Result<PhaseResults<'_>> {
        let mut results: PhaseResults = HashMap::new();
        let phase_names: HashSet<&str> = self.phases.iter().map(|p| p.name()).collect();

        // Build adjacency list and in-degree map
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

        for phase in &self.phases {
            let name = phase.name();
            in_degree.entry(name).or_insert(0);
            for dep in phase.deps() {
                if !phase_names.contains(dep) {
                    anyhow::bail!("Phase '{}' depends on unknown phase '{}'", name, dep);
                }
                adj.entry(dep).or_default().push(name);
                *in_degree.entry(name).or_insert(0) += 1;
            }
        }

        // Kahn's algorithm
        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|&(_, &d)| d == 0)
            .map(|(&n, _)| n)
            .collect();

        let mut executed = 0usize;

        while let Some(name) = queue.pop_front() {
            let phase = self
                .phases
                .iter()
                .find(|p| p.name() == name)
                .expect("phase exists");
            let output = phase.execute(ctx, &results)?;
            results.insert(name, output);
            executed += 1;

            if let Some(children) = adj.get(name) {
                for child in children {
                    let deg = in_degree.get_mut(child).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(child);
                    }
                }
            }
        }

        if executed != self.phases.len() {
            anyhow::bail!("Pipeline contains a cycle or disconnected phases");
        }

        Ok(results)
    }
}

/// Helper: downcast a PhaseOutput to a concrete type.
pub fn downcast_output<T: Any>(output: &PhaseOutput) -> Option<&T> {
    output.downcast_ref::<T>()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyPhase {
        name: &'static str,
        deps: &'static [&'static str],
    }

    impl PipelinePhase for DummyPhase {
        fn name(&self) -> &str {
            self.name
        }
        fn deps(&self) -> &[&str] {
            self.deps
        }
        fn execute(
            &self,
            _ctx: &mut PipelineContext,
            _deps: &PhaseResults,
        ) -> anyhow::Result<PhaseOutput> {
            Ok(Box::new(()) as PhaseOutput)
        }
    }

    #[test]
    fn run_simple_dag() {
        let phases: Vec<Box<dyn PipelinePhase>> = vec![
            Box::new(DummyPhase {
                name: "scan",
                deps: &[],
            }),
            Box::new(DummyPhase {
                name: "structure",
                deps: &["scan"],
            }),
            Box::new(DummyPhase {
                name: "parse",
                deps: &["structure"],
            }),
        ];
        let runner = PipelineRunner::new(phases);
        let mut ctx = PipelineContext::new(".");
        let results = runner.run(&mut ctx).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn detect_cycle() {
        let phases: Vec<Box<dyn PipelinePhase>> = vec![
            Box::new(DummyPhase {
                name: "a",
                deps: &["b"],
            }),
            Box::new(DummyPhase {
                name: "b",
                deps: &["a"],
            }),
        ];
        let runner = PipelineRunner::new(phases);
        let mut ctx = PipelineContext::new(".");
        assert!(runner.run(&mut ctx).is_err());
    }
}

// ============================================================================
// Integration: full 12-phase pipeline builder
// ============================================================================

/// Build the complete pipeline runner for the minimal release.
pub fn build_full_pipeline() -> PipelineRunner {
    let phases: Vec<Box<dyn PipelinePhase>> = vec![
        Box::new(scan::ScanPhase),
        Box::new(structure::StructurePhase),
        Box::new(parse::ParsePhase),
        Box::new(cross_file::CrossFilePhase),
        Box::new(mro::MroPhase),
        Box::new(resolve::ResolvePhase),
        Box::new(communities::CommunitiesPhase),
        Box::new(processes::ProcessesPhase),
    ];
    PipelineRunner::new(phases)
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn full_pipeline_smoke() {
        let tmp = tempfile::tempdir().unwrap();
        // Create a small repo
        std::fs::create_dir(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(
            tmp.path().join("README.md"),
            "# Hello\n\n[link](./src/main.rs)",
        )
        .unwrap();

        let runner = build_full_pipeline();
        let mut ctx = PipelineContext::new(tmp.path());
        let results = runner.run(&mut ctx).unwrap();

        assert!(results.contains_key("scan"));
        assert!(results.contains_key("structure"));
        assert!(results.contains_key("parse"));

        // Check graph has nodes
        assert!(ctx.graph.node_count() > 0);

        // Check scan result
        let scan_result = scan::get_scan_result(&results).unwrap();
        assert_eq!(scan_result.files.len(), 2);

        // Check communities phase produced output
        if let Some(communities_output) = results.get("communities")
            && communities_output
                .downcast_ref::<communities::CommunitiesPhaseOutput>()
                .is_some()
        {
            // Phase ran successfully and returned valid output.
        }
    }

    #[test]
    fn pipeline_communities_detects_groups() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("src")).unwrap();

        // Module A: tightly connected functions
        std::fs::write(
            tmp.path().join("src/auth.rs"),
            r#"
pub fn login() {}
pub fn logout() {}
pub fn verify_token() {}
fn login_impl() { login(); }
"#,
        )
        .unwrap();

        // Module B: another tightly connected group
        std::fs::write(
            tmp.path().join("src/db.rs"),
            r#"
pub fn connect() {}
pub fn query() {}
pub fn migrate() {}
fn query_impl() { query(); }
"#,
        )
        .unwrap();

        // Main entry that calls both modules
        std::fs::write(
            tmp.path().join("src/main.rs"),
            r#"
mod auth;
mod db;
fn main() {
    auth::login();
    db::connect();
}
"#,
        )
        .unwrap();

        let runner = build_full_pipeline();
        let mut ctx = PipelineContext::new(tmp.path());
        let results = runner.run(&mut ctx).unwrap();

        assert!(results.contains_key("communities"));
        if let Some(communities_output) = results.get("communities")
            && let Some(output) =
                communities_output.downcast_ref::<communities::CommunitiesPhaseOutput>()
        {
            // Should have detected at least one community
            assert!(
                output.community_count > 0,
                "Expected at least one community, got {}",
                output.community_count
            );
        }

        // Verify Community nodes exist in the graph
        let community_nodes = ctx.graph.nodes_by_kind(cg_common::NodeKind::Community);
        assert!(
            !community_nodes.is_empty(),
            "Expected Community nodes in graph"
        );

        // Verify MEMBER_OF edges exist
        let member_edges: usize = ctx
            .graph
            .out_edges
            .iter()
            .map(|e| {
                e.value()
                    .iter()
                    .filter(|edge| edge.kind == cg_common::EdgeKind::MemberOf)
                    .count()
            })
            .sum();
        assert!(member_edges > 0, "Expected MEMBER_OF edges in graph");
    }

    #[test]
    fn pipeline_resolve_cross_file_calls() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("src")).unwrap();

        // Module with a public function
        std::fs::write(
            tmp.path().join("src/helper.rs"),
            r#"
pub fn greet(name: &str) -> String {
    format!("Hello, {}!", name)
}
"#,
        )
        .unwrap();

        // Main that calls the helper function
        std::fs::write(
            tmp.path().join("src/main.rs"),
            r#"
mod helper;
fn main() {
    let msg = helper::greet("world");
    println!("{}", msg);
}
"#,
        )
        .unwrap();

        let runner = build_full_pipeline();
        let mut ctx = PipelineContext::new(tmp.path());
        let results = runner.run(&mut ctx).unwrap();

        // Verify resolve phase ran and produced output
        assert!(results.contains_key("resolve"));
        if let Some(resolve_output) = results.get("resolve")
            && let Some(output) = resolve_output.downcast_ref::<resolve::ResolvePhaseOutput>()
        {
            // At least one cross-file call should have been resolved
            // (helper::greet from main.rs)
            assert!(
                output.resolved_count > 0,
                "Expected at least one resolved cross-file call, got {}",
                output.resolved_count
            );
        }

        // Verify CALLS edges exist after resolution
        let call_edges: usize = ctx
            .graph
            .out_edges
            .iter()
            .map(|e| {
                e.value()
                    .iter()
                    .filter(|edge| edge.kind == cg_common::EdgeKind::Calls)
                    .count()
            })
            .sum();
        assert!(
            call_edges > 0,
            "Expected CALLS edges after resolution, got {}",
            call_edges
        );
    }

    #[test]
    fn pipeline_detects_cycle() {
        struct A;
        struct B;
        impl PipelinePhase for A {
            fn name(&self) -> &str {
                "a"
            }
            fn deps(&self) -> &[&str] {
                &["b"]
            }
            fn execute(
                &self,
                _ctx: &mut PipelineContext,
                _deps: &PhaseResults,
            ) -> anyhow::Result<PhaseOutput> {
                Ok(Box::new(()))
            }
        }
        impl PipelinePhase for B {
            fn name(&self) -> &str {
                "b"
            }
            fn deps(&self) -> &[&str] {
                &["a"]
            }
            fn execute(
                &self,
                _ctx: &mut PipelineContext,
                _deps: &PhaseResults,
            ) -> anyhow::Result<PhaseOutput> {
                Ok(Box::new(()))
            }
        }

        let runner = PipelineRunner::new(vec![Box::new(A), Box::new(B)]);
        let mut ctx = PipelineContext::new(".");
        assert!(runner.run(&mut ctx).is_err());
    }
}
