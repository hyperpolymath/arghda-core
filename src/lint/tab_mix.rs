// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! `tab-mix` (warn) — flag tabs used for indentation.
//!
//! Agda's layout rule and the estate style use spaces; a stray tab in
//! leading whitespace breaks alignment and, inside a layout block, can
//! change how the file parses. Reported once per file (first offending
//! line) to keep the signal quiet.

use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub struct TabMix;

impl LintRule for TabMix {
    fn name(&self) -> &'static str {
        "tab-mix"
    }

    fn run(&self, file: &Path, _ctx: &LintContext<'_>, report: &mut LintReport) -> Result<()> {
        let contents =
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        for (i, line) in contents.lines().enumerate() {
            let leading_has_tab = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .any(|c| c == '\t');
            if leading_has_tab {
                report.push(Diagnostic {
                    rule: self.name().to_string(),
                    severity: Severity::Warn,
                    file: file.to_path_buf(),
                    message: "tab in leading whitespace (Agda style is spaces)".to_string(),
                    line: Some(i + 1),
                });
                break; // one report per file is enough signal
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::run_lints;
    use std::path::PathBuf;

    fn lint_str(body: &str) -> LintReport {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), body).unwrap();
        let roots: [PathBuf; 0] = [];
        let ctx = LintContext {
            include_root: tmp.path().parent().unwrap(),
            entry_modules: &roots,
        };
        let rules: Vec<Box<dyn LintRule>> = vec![Box::new(TabMix)];
        run_lints(tmp.path(), &ctx, &rules).unwrap()
    }

    #[test]
    fn leading_tab_is_warned_not_blocked() {
        let r = lint_str("module M where\n\tx = 1\n");
        assert!(!r.has_hard_block());
        assert_eq!(r.warns().count(), 1);
        assert_eq!(r.diagnostics[0].line, Some(2));
    }

    #[test]
    fn space_indented_is_clean() {
        let r = lint_str("module M where\n  x = 1\n");
        assert!(r.diagnostics.is_empty());
    }
}
