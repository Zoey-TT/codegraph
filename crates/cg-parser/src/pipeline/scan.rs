use super::{PhaseOutput, PhaseResults, PipelineContext, PipelinePhase, downcast_output};
use crate::scanner::{ScanOptions, ScanResult, scan_directory};

pub struct ScanPhase;

impl PipelinePhase for ScanPhase {
    fn name(&self) -> &str {
        "scan"
    }

    fn deps(&self) -> &[&str] {
        &[]
    }

    fn execute(
        &self,
        ctx: &mut PipelineContext,
        _deps: &PhaseResults,
    ) -> anyhow::Result<PhaseOutput> {
        let options = ScanOptions::default();
        let result = scan_directory(&ctx.repo_path, &options)?;
        Ok(Box::new(result))
    }
}

/// Fetch the `ScanResult` from phase results.
pub fn get_scan_result<'a>(results: &'a PhaseResults<'a>) -> Option<&'a ScanResult> {
    results
        .get("scan")
        .and_then(|o| downcast_output::<ScanResult>(o))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn scan_phase_smoke() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn main() {}").unwrap();
        std::fs::write(tmp.path().join("b.ts"), "const x = 1;").unwrap();

        let mut ctx = PipelineContext::new(tmp.path());
        let phase = ScanPhase;
        let output = phase.execute(&mut ctx, &HashMap::new()).unwrap();
        let result = downcast_output::<ScanResult>(&output).unwrap();

        assert_eq!(result.files.len(), 2);
        assert_eq!(result.total_seen, 2);
    }
}
