// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! `unjustified-postulate` — flag `postulate` openers that lack an
//! adjacent `-- JUSTIFY:` rationale.
//!
//! Postulates are the canonical way a "proof" hides a hole, so the estate
//! discipline (echo-types / absolute-zero) is: a postulate is permitted
//! only if it is explicitly justified. This rule makes the bare case a
//! hard-block, matching `arghda-spec.adoc`'s rule list.

use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub struct UnjustifiedPostulate;

impl LintRule for UnjustifiedPostulate {
    fn name(&self) -> &'static str {
        "unjustified-postulate"
    }

    fn run(&self, file: &Path, _ctx: &LintContext<'_>, report: &mut LintReport) -> Result<()> {
        let contents =
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let lines: Vec<&str> = contents.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            if !is_postulate_opener(line) {
                continue;
            }
            if !justified_above(&lines, i) {
                report.push(Diagnostic {
                    rule: self.name().to_string(),
                    severity: Severity::HardBlock,
                    file: file.to_path_buf(),
                    message: "`postulate` without an adjacent `-- JUSTIFY:` comment".to_string(),
                    line: Some(i + 1),
                });
            }
        }
        Ok(())
    }
}

/// A line whose first token is `postulate` (a block opener `postulate` on
/// its own line, or an inline `postulate name : T`). Comment lines are
/// excluded.
fn is_postulate_opener(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("--") {
        return false;
    }
    t == "postulate" || t.starts_with("postulate ") || t.starts_with("postulate\t")
}

/// True if the nearest non-blank line above `idx` is a `-- JUSTIFY:` comment.
fn justified_above(lines: &[&str], idx: usize) -> bool {
    for j in (0..idx).rev() {
        let t = lines[j].trim();
        if t.is_empty() {
            continue;
        }
        return t.starts_with("-- JUSTIFY:");
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::run_lints;

    fn lint_str(body: &str) -> LintReport {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), body).unwrap();
        let roots = [tmp.path().to_path_buf()];
        let ctx = LintContext {
            include_root: tmp.path().parent().unwrap(),
            entry_modules: &roots,
        };
        let rules: Vec<Box<dyn LintRule>> = vec![Box::new(UnjustifiedPostulate)];
        run_lints(tmp.path(), &ctx, &rules).unwrap()
    }

    #[test]
    fn bare_postulate_is_flagged() {
        let r = lint_str("module M where\npostulate\n  foo : Set\n");
        assert!(r.has_hard_block());
        assert_eq!(r.diagnostics[0].line, Some(2));
    }

    #[test]
    fn justified_postulate_is_clean() {
        let r = lint_str(
            "module M where\n-- JUSTIFY: classical axiom, see ADR-007\npostulate\n  lem : Set\n",
        );
        assert!(!r.has_hard_block(), "got: {:?}", r.diagnostics);
    }

    #[test]
    fn inline_postulate_without_justify_is_flagged() {
        let r = lint_str("module M where\npostulate foo : Set\n");
        assert!(r.has_hard_block());
    }

    #[test]
    fn word_postulated_is_not_an_opener() {
        let r = lint_str("module M where\n-- this is postulated elsewhere\nfoo : Set\n");
        assert!(!r.has_hard_block());
    }
}
