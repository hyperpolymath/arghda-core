//! `escape-hatch` (warn) — surface soundness escape hatches.
//!
//! Flags termination-checker overrides (`{-# TERMINATING #-}`,
//! `NON_TERMINATING`, `NO_TERMINATION_CHECK`) and the trust primitives
//! `believe_me` / `primTrustMe`. These are sometimes legitimately budgeted
//! (see echo-types' quarantine discipline), so this is a *warn*, not a
//! hard-block: it makes the hatch visible to the operator / visual layer
//! without blocking promotion. The postulate case is handled separately by
//! `unjustified-postulate` (hard-block when there is no `-- JUSTIFY:`).

use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub struct EscapeHatch;

const TERMINATION_PRAGMAS: &[&str] = &["TERMINATING", "NON_TERMINATING", "NO_TERMINATION_CHECK"];
const TRUST_PRIMS: &[&str] = &["believe_me", "primTrustMe"];

impl LintRule for EscapeHatch {
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
            // Termination-checker override pragmas.
            if trimmed.starts_with("{-#") {
                for p in TERMINATION_PRAGMAS {
                    if line.contains(p) {
                        report.push(warn(
                            self.name(),
                            file,
                            i + 1,
                            format!("termination escape pragma `{p}`"),
                        ));
                    }
                }
            }
            // Trust primitives, ignoring any trailing line comment.
            let code = line.split(" --").next().unwrap_or(line);
            for prim in TRUST_PRIMS {
                if has_token(code, prim) {
                    report.push(warn(
                        self.name(),
                        file,
                        i + 1,
                        format!("trust primitive `{prim}`"),
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

/// `tok` appears in `s` as a whitespace/paren-delimited token (so a longer
/// identifier like `believe_me_helper` does not match).
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
        let rules: Vec<Box<dyn LintRule>> = vec![Box::new(EscapeHatch)];
        run_lints(tmp.path(), &ctx, &rules).unwrap()
    }

    #[test]
    fn terminating_pragma_is_warned() {
        let r = lint_str("module M where\n{-# TERMINATING #-}\nf : Set\n");
        assert!(!r.has_hard_block());
        assert_eq!(r.warns().count(), 1);
        assert!(r.diagnostics[0].message.contains("TERMINATING"));
    }

    #[test]
    fn believe_me_token_is_warned() {
        let r = lint_str("module M where\nx = believe_me 0\n");
        assert_eq!(r.warns().count(), 1);
        assert!(r.diagnostics[0].message.contains("believe_me"));
    }

    #[test]
    fn believe_me_in_comment_is_ignored() {
        let r = lint_str("module M where\nx = 0 -- todo: avoid believe_me here\n");
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn longer_identifier_does_not_match() {
        let r = lint_str("module M where\nbelieve_me_helper = 0\n");
        assert!(r.diagnostics.is_empty());
    }

    #[test]
    fn clean_file_is_silent() {
        let r = lint_str("module M where\nf : Set\nf = Set\n");
        assert!(r.diagnostics.is_empty());
    }
}
