use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use crate::graph::{module_name_of, transitive_imports};
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

        // The entry module itself is never an orphan of itself.
        if Some(module.as_str()) == module_name_of(ctx.entry_module, ctx.include_root).as_deref() {
            return Ok(());
        }

        let reachable =
            transitive_imports(ctx.entry_module, ctx.include_root).with_context(|| {
                format!(
                    "computing transitive imports from {}",
                    ctx.entry_module.display()
                )
            })?;

        if !reachable.contains(&module) {
            report.push(Diagnostic {
                rule: self.name().to_string(),
                severity: Severity::HardBlock,
                file: file.to_path_buf(),
                message: format!(
                    "module `{}` is not reachable via imports from `{}`",
                    module,
                    ctx.entry_module.display()
                ),
                line: None,
            });
        }

        Ok(())
    }
}
