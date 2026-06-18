use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use crate::graph::{module_name_of, reachable_from_roots};
use anyhow::{Context, Result};
use std::path::Path;

pub struct OrphanModule;

impl LintRule for OrphanModule {
    fn name(&self) -> &'static str {
        "orphan-module"
    }

    fn run(&self, file: &Path, ctx: &LintContext<'_>, report: &mut LintReport) -> Result<()> {
        let Some(module) = module_name_of(file, ctx.include_root) else {
            return Ok(()); // file sits outside include_root; nothing to say
        };

        // A root module is never an orphan of itself.
        let is_root = ctx
            .entry_modules
            .iter()
            .any(|root| module_name_of(root, ctx.include_root).as_deref() == Some(module.as_str()));
        if is_root {
            return Ok(());
        }

        let reachable = reachable_from_roots(ctx.entry_modules, ctx.include_root)
            .context("computing reachability from root modules")?;

        if !reachable.contains(&module) {
            report.push(Diagnostic {
                rule: self.name().to_string(),
                severity: Severity::HardBlock,
                file: file.to_path_buf(),
                message: format!(
                    "module `{}` is not reachable via imports from any of the {} root module(s)",
                    module,
                    ctx.entry_modules.len()
                ),
                line: None,
            });
        }

        Ok(())
    }
}
