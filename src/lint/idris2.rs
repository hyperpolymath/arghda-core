// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Idris2 `escape-hatch` (warn) — surface soundness escape hatches.
//!
//! The Idris2 analogue of the Agda [`super::escape_hatch::EscapeHatch`]. It
//! shares the rule *name* `escape-hatch` deliberately: the reasoning graph
//! (`crate::reason`) caps any `escape-hatch` node at amber, so a `believe_me`
//! in Idris2 and a `believe_me` in Agda get the same honest treatment.
//!
//! Flags the Idris2 trust/escape primitives `believe_me`, `assert_total`,
//! `assert_smaller`, `idris_crash`, and the `%default partial` directive.
//! Idris2 `--check` exits 0 even when these are present, so without this rule
//! they would silently ride along in an otherwise-green verdict — exactly the
//! silent-failure class arghda exists to surface. Totality holes (`?name`)
//! and per-def `partial` modifiers are a documented follow-on.

use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub struct Idris2EscapeHatch;

const ESCAPE_TOKENS: &[&str] = &[
    "believe_me",
    "assert_total",
    "assert_smaller",
    "idris_crash",
];

impl LintRule for Idris2EscapeHatch {
    fn name(&self) -> &'static str {
        // Shared with the Agda rule on purpose — the reasoning graph's
        // lint→verdict cap keys on this name.
        "escape-hatch"
    }

    fn run(&self, file: &Path, _ctx: &LintContext<'_>, report: &mut LintReport) -> Result<()> {
        let contents =
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        for (i, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("--") {
                continue; // whole-line comment
            }
            // The `%default partial` directive turns off totality checking.
            if trimmed.starts_with("%default") && trimmed.contains("partial") {
                report.push(warn(
                    self.name(),
                    file,
                    i + 1,
                    "totality escape directive `%default partial`".to_string(),
                ));
            }
            // Trust/escape primitives, ignoring any trailing line comment.
            let code = line.split(" --").next().unwrap_or(line);
            for tok in ESCAPE_TOKENS {
                if has_token(code, tok) {
                    report.push(warn(
                        self.name(),
                        file,
                        i + 1,
                        format!("escape primitive `{tok}`"),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn warn(rule: &str, file: &Path, line: usize, message: String) -> Diagnostic {
    Diagnostic {
        rule: rule.to_string(),
        severity: Severity::Warn,
        file: file.to_path_buf(),
        message,
        line: Some(line),
    }
}

/// `tok` appears in `s` as a delimited token (so `assert_totally` or
/// `believe_me_helper` do not match).
fn has_token(s: &str, tok: &str) -> bool {
    s.split(|c: char| c.is_whitespace() || "(){};,".contains(c))
        .any(|w| w == tok)
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
        let rules: Vec<Box<dyn LintRule>> = vec![Box::new(Idris2EscapeHatch)];
        run_lints(tmp.path(), &ctx, &rules).unwrap()
    }

    #[test]
    fn believe_me_is_warned_under_the_escape_hatch_name() {
        let r = lint_str("module M\n\nx : Nat\nx = believe_me 0\n");
        assert_eq!(r.warns().count(), 1);
        assert_eq!(r.diagnostics[0].rule, "escape-hatch");
        assert!(r.diagnostics[0].message.contains("believe_me"));
    }

    #[test]
    fn assert_total_and_default_partial_are_warned() {
        let r = lint_str("%default partial\n\nf : Nat -> Nat\nf x = assert_total (f x)\n");
        assert_eq!(r.warns().count(), 2);
    }

    #[test]
    fn escape_token_in_comment_is_ignored() {
        let r = lint_str("module M\n\nx : Nat\nx = 0 -- avoid believe_me here\n");
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn longer_identifier_does_not_match() {
        let r = lint_str("module M\n\nbelieve_me_helper : Nat\nbelieve_me_helper = 0\n");
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn clean_idris_file_is_silent() {
        let r = lint_str("module M\n\ngreeting : String\ngreeting = \"hi\"\n");
        assert!(r.diagnostics.is_empty());
    }
}
