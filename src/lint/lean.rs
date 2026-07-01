// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Lean4 `escape-hatch` (warn) — surface soundness escape hatches.
//!
//! The Lean4 analogue of the Agda/Idris2 escape-hatch rules; shares the rule
//! *name* `escape-hatch` so the reasoning graph caps any hit at amber.
//!
//! Flags `sorry` and `admit` (proof holes), `native_decide` (trusts the
//! compiler's evaluation, outside the kernel) and `unsafe` (bypasses the
//! termination/positivity checks). Lean `--check`/elaboration exits 0 even
//! with `sorry` present (it is only a *warning*), so without this rule — and
//! without the `#print axioms` audit — such gaps ride along invisibly. That
//! is why the Lean backend's own verdict is at best `Unknown` on a green
//! elaboration absent the audit.

use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub struct LeanEscapeHatch;

const ESCAPE_TOKENS: &[&str] = &["sorry", "admit", "native_decide", "unsafe"];

impl LintRule for LeanEscapeHatch {
    fn name(&self) -> &'static str {
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

/// `tok` appears in `s` as a delimited token (so `sorryAx_helper` or a
/// namespaced `Foo.admit` do not spuriously match the bare keyword).
fn has_token(s: &str, tok: &str) -> bool {
    s.split(|c: char| c.is_whitespace() || "(){};,.".contains(c))
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
        let rules: Vec<Box<dyn LintRule>> = vec![Box::new(LeanEscapeHatch)];
        run_lints(tmp.path(), &ctx, &rules).unwrap()
    }

    #[test]
    fn sorry_is_warned_under_escape_hatch() {
        let r = lint_str("theorem t : 1 = 1 := by sorry\n");
        assert_eq!(r.warns().count(), 1);
        assert_eq!(r.diagnostics[0].rule, "escape-hatch");
        assert!(r.diagnostics[0].message.contains("sorry"));
    }

    #[test]
    fn native_decide_and_unsafe_are_warned() {
        let r = lint_str("unsafe def f : Nat := 0\nexample : P := by native_decide\n");
        assert_eq!(r.warns().count(), 2);
    }

    #[test]
    fn escape_token_in_comment_is_ignored() {
        let r = lint_str("def x : Nat := 0 -- TODO: no sorry here\n");
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn clean_lean_file_is_silent() {
        let r = lint_str("theorem t : 1 = 1 := rfl\n");
        assert!(r.diagnostics.is_empty());
    }
}
