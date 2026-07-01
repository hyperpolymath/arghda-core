// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Coq / Rocq `escape-hatch` (warn) — surface soundness escape hatches.
//!
//! The Coq analogue of the Agda/Idris2/Lean escape-hatch rules; shares the
//! rule *name* `escape-hatch` so the reasoning graph caps any hit at amber.
//!
//! Flags the unambiguous holes: `Admitted`/`admit` (a proof closed as an
//! axiom / left as a placeholder) and `Axiom`/`Conjecture` (declared, not
//! proved). `coqc` compiles all of these with exit 0, so without this warn —
//! and without the backend's Section-aware postulate classification — they
//! ride along invisibly. `Parameter`/`Variable`/`Hypothesis` are deliberately
//! NOT flagged here: they are legitimate scaffold shapes far more often than
//! not, and the genuine module-level `Parameter` postulate is caught by the
//! backend's `count_genuine_postulates` (Section-aware) instead.

use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub struct CoqEscapeHatch;

const ESCAPE_TOKENS: &[&str] = &["Admitted", "admit", "Axiom", "Conjecture"];

impl LintRule for CoqEscapeHatch {
    fn name(&self) -> &'static str {
        "escape-hatch"
    }

    fn run(&self, file: &Path, _ctx: &LintContext<'_>, report: &mut LintReport) -> Result<()> {
        let contents =
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        for (i, raw) in contents.lines().enumerate() {
            let trimmed = raw.trim_start();
            if trimmed.starts_with("(*") {
                continue; // whole-line comment (common case)
            }
            let code = strip_line_comments(raw);
            for tok in ESCAPE_TOKENS {
                if has_token(&code, tok) {
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

/// Remove balanced `(* … *)` spans; an unmatched `(*` truncates the rest of
/// the line.
fn strip_line_comments(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut depth = 0u32;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'(' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
            continue;
        }
        if depth > 0 && i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b')' {
            depth -= 1;
            i += 2;
            continue;
        }
        if depth == 0 {
            out.push(bytes[i] as char);
        }
        i += 1;
    }
    out
}

/// `tok` appears in `s` as a delimited token (so `admittedly` or `Foo.admit`
/// do not match the bare keyword).
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
        let rules: Vec<Box<dyn LintRule>> = vec![Box::new(CoqEscapeHatch)];
        run_lints(tmp.path(), &ctx, &rules).unwrap()
    }

    #[test]
    fn admitted_is_warned() {
        let r = lint_str("Theorem t : True.\nProof.\nAdmitted.\n");
        assert_eq!(r.warns().count(), 1);
        assert_eq!(r.diagnostics[0].rule, "escape-hatch");
        assert!(r.diagnostics[0].message.contains("Admitted"));
    }

    #[test]
    fn axiom_and_admit_tactic_are_warned() {
        let r = lint_str(
            "Axiom em : forall P : Prop, P \\/ ~P.\nTheorem t : True. Proof. admit. Defined.\n",
        );
        assert_eq!(r.warns().count(), 2);
    }

    #[test]
    fn escape_token_in_comment_is_ignored() {
        let r = lint_str("Definition x := 0. (* no Admitted here *)\n");
        assert!(r.diagnostics.is_empty());
        let r2 = lint_str("(* Axiom foo : bar. *)\n");
        assert!(r2.diagnostics.is_empty());
    }

    #[test]
    fn clean_coq_file_is_silent() {
        let r = lint_str("Theorem t : True.\nProof.\nexact I.\nQed.\n");
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn parameter_is_not_flagged_by_the_lint() {
        // Parameter is left to the backend's Section-aware classifier.
        let r = lint_str("Parameter State : Type.\n");
        assert!(r.diagnostics.is_empty());
    }
}
