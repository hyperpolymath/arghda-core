use crate::diagnostic::LintReport;
use anyhow::Result;
use std::path::Path;

pub mod orphan_module;
pub mod safe_pragma;

/// Context handed to every rule.
#[derive(Clone, Debug)]
pub struct LintContext<'a> {
    /// Agda include root; `.agda` files' module names are computed
    /// relative to this path.
    pub include_root: &'a Path,
    /// Path to the `All.agda` (or equivalent) entry module.
    pub entry_module: &'a Path,
}

pub trait LintRule: Send + Sync {
    fn name(&self) -> &'static str;
    fn run(&self, file: &Path, ctx: &LintContext<'_>, report: &mut LintReport) -> Result<()>;
}

pub fn default_rules() -> Vec<Box<dyn LintRule>> {
    vec![
        Box::new(safe_pragma::SafePragma),
        Box::new(orphan_module::OrphanModule),
    ]
}

pub fn run_lints(
    file: &Path,
    ctx: &LintContext<'_>,
    rules: &[Box<dyn LintRule>],
) -> Result<LintReport> {
    let mut report = LintReport::new(file.to_path_buf());
    for rule in rules {
        rule.run(file, ctx, &mut report)?;
    }
    Ok(report)
}
