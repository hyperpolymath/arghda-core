// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Isabelle/HOL `escape-hatch` (warn) — surface soundness escape hatches.
//!
//! The Isabelle analogue of the Agda/Idris2/Lean/Coq escape-hatch rules;
//! shares the rule *name* `escape-hatch` so the reasoning graph caps any hit
//! at amber.
//!
//! Flags `sorry` (admits a proposition without proof), `oops` (abandons a
//! proof mid-attempt) and `axiomatization` (introduces unverified axioms).
//! `isabelle build -o quick_and_dirty` compiles a theory containing `sorry`
//! with exit 0, so without this warn — and without the backend's source-level
//! classification — such gaps ride along invisibly. Block comments `(* … *)`
//! are stripped so a "no sorry here" comment does not match; prose blocks
//! (`text ‹…›`) are a documented minor false-positive risk.

use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub struct IsabelleEscapeHatch;

const ESCAPE_TOKENS: &[&str] = &["sorry", "oops", "axiomatization"];

impl LintRule for IsabelleEscapeHatch {
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

/// `tok` appears in `s` as a delimited token (so `sorryish` does not match).
fn has_token(s: &str, tok: &str) -> bool {
    s.split(|c: char| c.is_whitespace() || "(){}[];,.".contains(c))
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
        let rules: Vec<Box<dyn LintRule>> = vec![Box::new(IsabelleEscapeHatch)];
        run_lints(tmp.path(), &ctx, &rules).unwrap()
    }

    #[test]
    fn sorry_is_warned() {
        let r = lint_str("theory T imports Main begin\nlemma l: \"True\" sorry\nend\n");
        assert_eq!(r.warns().count(), 1);
        assert_eq!(r.diagnostics[0].rule, "escape-hatch");
        assert!(r.diagnostics[0].message.contains("sorry"));
    }

    #[test]
    fn oops_and_axiomatization_are_warned() {
        let r = lint_str(
            "theory T imports Main begin\naxiomatization where foo: \"P\"\nlemma l: \"Q\" oops\nend\n",
        );
        assert_eq!(r.warns().count(), 2);
    }

    #[test]
    fn escape_token_in_comment_is_ignored() {
        let r = lint_str(
            "theory T imports Main begin\ndefinition x where \"x = 0\" (* no sorry *)\nend\n",
        );
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn clean_theory_is_silent() {
        let r = lint_str("theory T imports Main begin\nlemma l: \"True\" by simp\nend\n");
        assert!(r.diagnostics.is_empty());
    }
}
