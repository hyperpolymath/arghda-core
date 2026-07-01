// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! `missing-safe-pragma` (hard-block) — require the soundness OPTIONS pragma.
//!
//! Parameterised by the flags a file's `{-# OPTIONS … #-}` must carry.
//! Standard Agda demands `--safe --without-K`; Cubical Agda demands
//! `--cubical --safe` (cubical is incompatible with `--without-K`, so
//! requiring it there would be wrong). Both profiles share the rule name so
//! the reasoning graph / DAG treat them uniformly.

use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Requires a specific set of flags in an OPTIONS pragma.
pub struct SafePragma {
    required: &'static [&'static str],
}

const HEAD_LINES_SCANNED: usize = 30;

impl SafePragma {
    /// Standard `--safe --without-K` Agda.
    pub const fn standard() -> Self {
        Self {
            required: &["--safe", "--without-K"],
        }
    }

    /// Cubical Agda: `--cubical --safe` (no `--without-K`).
    pub const fn cubical() -> Self {
        Self {
            required: &["--cubical", "--safe"],
        }
    }
}

impl LintRule for SafePragma {
    fn name(&self) -> &'static str {
        "missing-safe-pragma"
    }

    fn run(&self, file: &Path, _ctx: &LintContext<'_>, report: &mut LintReport) -> Result<()> {
        let contents =
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;

        let mut seen = vec![false; self.required.len()];
        for line in contents.lines().take(HEAD_LINES_SCANNED) {
            if !line.trim_start().starts_with("{-#") {
                continue;
            }
            if line.contains("OPTIONS") {
                for (i, flag) in self.required.iter().enumerate() {
                    if line.contains(flag) {
                        seen[i] = true;
                    }
                }
            }
        }

        let missing: Vec<&str> = self
            .required
            .iter()
            .zip(&seen)
            .filter(|(_, &s)| !s)
            .map(|(flag, _)| *flag)
            .collect();

        if !missing.is_empty() {
            report.push(Diagnostic {
                rule: self.name().to_string(),
                severity: Severity::HardBlock,
                file: file.to_path_buf(),
                message: format!(
                    "missing {} pragma in first {} lines",
                    missing.join(" and "),
                    HEAD_LINES_SCANNED
                ),
                line: None,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::run_lints;
    use std::path::PathBuf;

    fn lint_with(rule: SafePragma, body: &str) -> LintReport {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), body).unwrap();
        let roots: [PathBuf; 0] = [];
        let ctx = LintContext {
            include_root: tmp.path().parent().unwrap(),
            entry_modules: &roots,
        };
        let rules: Vec<Box<dyn LintRule>> = vec![Box::new(rule)];
        run_lints(tmp.path(), &ctx, &rules).unwrap()
    }

    #[test]
    fn standard_requires_safe_and_without_k() {
        let ok = lint_with(
            SafePragma::standard(),
            "{-# OPTIONS --safe --without-K #-}\nmodule M where\n",
        );
        assert!(ok.diagnostics.is_empty());
        let bad = lint_with(SafePragma::standard(), "module M where\n");
        assert!(bad.has_hard_block());
    }

    #[test]
    fn standard_rejects_a_cubical_file() {
        // A cubical file lacks --without-K, so the standard rule flags it.
        let r = lint_with(
            SafePragma::standard(),
            "{-# OPTIONS --cubical --safe #-}\nmodule M where\n",
        );
        assert!(r.has_hard_block());
        assert!(r.diagnostics[0].message.contains("--without-K"));
    }

    #[test]
    fn cubical_accepts_cubical_and_rejects_without_k_only() {
        let ok = lint_with(
            SafePragma::cubical(),
            "{-# OPTIONS --cubical --safe #-}\nmodule M where\n",
        );
        assert!(ok.diagnostics.is_empty(), "cubical pragma must pass");
        // A standard --safe --without-K file lacks --cubical: flagged here.
        let bad = lint_with(
            SafePragma::cubical(),
            "{-# OPTIONS --safe --without-K #-}\nmodule M where\n",
        );
        assert!(bad.has_hard_block());
        assert!(bad.diagnostics[0].message.contains("--cubical"));
    }
}
