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
//! `assert_smaller`, `idris_crash`, the `%default partial` directive, a
//! per-definition `partial` modifier (opts that definition out of totality
//! checking), and totality holes `?name` (incomplete terms/proofs). Idris2
//! `--check` exits 0 even when these are present, so without this rule they
//! would silently ride along in an otherwise-green verdict — exactly the
//! silent-failure class arghda exists to surface.

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
            // Everything below ignores a trailing line comment.
            let code = line.split(" --").next().unwrap_or(line);
            // The `%default partial` directive turns off totality checking
            // file-wide.
            if trimmed.starts_with("%default") && trimmed.contains("partial") {
                report.push(warn(
                    self.name(),
                    file,
                    i + 1,
                    "totality escape directive `%default partial`".to_string(),
                ));
            } else if has_token(code, "partial") {
                // A per-definition `partial` modifier opts that definition out
                // of totality checking (distinct from the file-wide directive
                // above, which is already reported).
                report.push(warn(
                    self.name(),
                    file,
                    i + 1,
                    "totality opt-out `partial` modifier".to_string(),
                ));
            }
            // Trust/escape primitives.
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
            // Totality holes `?name` — incomplete terms/proofs that `--check`
            // defers rather than rejects. String literals are blanked first so
            // a `"?x"` inside a string does not match.
            for hole in holes(&strip_strings(code)) {
                report.push(warn(
                    self.name(),
                    file,
                    i + 1,
                    format!("totality hole `?{hole}`"),
                ));
            }
        }
        Ok(())
    }
}

/// Blank out double-quoted string literals (content → spaces), honouring `\"`
/// escapes, so a `?name` inside a string is not mistaken for a hole.
fn strip_strings(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_str = false;
    let mut escaped = false;
    for c in line.chars() {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            out.push(' ');
        } else if c == '"' {
            in_str = true;
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// The hole names in `code`: a `?` that begins a token (start of line or after
/// whitespace / an opening bracket / a separator / `=`) and is followed by an
/// identifier. This distinguishes a hole `?goal` from a user operator like
/// `<?>` (where `?` is surrounded by symbol chars).
fn holes(code: &str) -> Vec<String> {
    let b = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'?' {
            let prev_ok = i == 0
                || matches!(
                    b[i - 1],
                    b' ' | b'\t' | b'(' | b'[' | b'{' | b',' | b';' | b'='
                );
            let next_ok = i + 1 < b.len() && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_');
            if prev_ok && next_ok {
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'\'')
                {
                    j += 1;
                }
                out.push(code[i + 1..j].to_string());
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
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

    #[test]
    fn per_def_partial_modifier_is_warned() {
        let r = lint_str("module M\n\npartial\nloop : Nat -> Nat\nloop x = loop x\n");
        assert_eq!(r.warns().count(), 1);
        assert!(r.diagnostics[0].message.contains("partial"));
    }

    #[test]
    fn totality_hole_is_warned() {
        let r = lint_str("module M\n\nf : Nat\nf = ?rhs\n");
        assert_eq!(r.warns().count(), 1);
        assert_eq!(r.diagnostics[0].rule, "escape-hatch");
        assert!(r.diagnostics[0].message.contains("?rhs"));
    }

    #[test]
    fn hole_inside_a_string_is_not_flagged() {
        let r = lint_str("module M\n\nmsg : String\nmsg = \"what is ?this\"\n");
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn question_operator_is_not_a_hole() {
        // A user operator like `<?>` has `?` between symbol chars — not a hole.
        let r = lint_str("module M\n\n(<?>) : Maybe a -> a -> a\n");
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn total_modifier_is_not_flagged() {
        let r = lint_str("module M\n\ntotal\ng : Nat -> Nat\ng x = x\n");
        assert!(r.diagnostics.is_empty());
    }
}
