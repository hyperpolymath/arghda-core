// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! The Agda backend — arghda's first (and v0.1 reference) [`Backend`].
//!
//! Owns the `agda` shell-out; delegates import-graph parsing to the
//! Agda-specific free functions in [`crate::graph`] and the lint pack to
//! [`crate::lint`]. Assistant model: `agda -i <root> <file>`; exit 0 →
//! [`Verdict::Proven`], a run that errors → [`Verdict::Error`], an absent
//! binary → [`Verdict::Unavailable`]. Exit-code only — arghda never claims
//! a result Agda did not return. `Admitted`/`Postulated` are amber overlays
//! surfaced by the lint pack (they are not process facts), so `check_file`
//! deliberately does not manufacture them from a green exit.

use super::{Backend, BackendKind, Outcome, Verdict};
use crate::graph;
use crate::lint::{rules_with_config, LintRule, RuleConfig};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

const TAIL_LINES: usize = 40;

/// The Agda proof assistant.
#[derive(Clone, Copy, Debug, Default)]
pub struct Agda;

impl Backend for Agda {
    fn name(&self) -> &'static str {
        "agda"
    }

    fn kind(&self) -> BackendKind {
        BackendKind::Assistant
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["agda"]
    }

    fn safe_mode(&self) -> Option<&'static str> {
        Some("--safe --without-K")
    }

    /// Typecheck `file` with `include_root` on the search path
    /// (`agda -i <include_root> <file>`).
    fn check_file(&self, file: &Path, include_root: &Path) -> Result<Outcome> {
        let output = Command::new("agda")
            .arg("-i")
            .arg(include_root)
            .arg(file)
            .output();

        match output {
            Ok(out) => {
                let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
                let ok = out.status.success();
                Ok(Outcome {
                    available: true,
                    exit_code: out.status.code(),
                    ok,
                    output_tail: tail(&combined, TAIL_LINES),
                    kind: BackendKind::Assistant,
                    // Exit-code only: 0 is the sole signal Agda gives that
                    // means "typechecked". Admitted/Postulated are
                    // lint-derived overlays, not process facts.
                    verdict: if ok { Verdict::Proven } else { Verdict::Error },
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Outcome::unavailable(BackendKind::Assistant))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn module_name_of(&self, file: &Path, include_root: &Path) -> Option<String> {
        graph::module_name_of(file, include_root)
    }

    fn module_to_path(&self, module: &str, include_root: &Path) -> PathBuf {
        graph::module_to_path(module, include_root)
    }

    fn direct_imports(&self, file: &Path) -> Result<Vec<String>> {
        graph::direct_imports(file)
    }

    fn discover_roots(&self, include_root: &Path) -> Vec<PathBuf> {
        graph::discover_roots(include_root)
    }

    fn lint_rules(&self, cfg: &RuleConfig) -> Result<Vec<Box<dyn LintRule>>> {
        rules_with_config(cfg)
    }
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_last_n_lines() {
        assert_eq!(tail("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(tail("only", 5), "only");
        assert_eq!(tail("", 3), "");
    }

    #[test]
    fn agda_backend_identity() {
        assert_eq!(Agda.name(), "agda");
        assert_eq!(Agda.kind(), BackendKind::Assistant);
        assert_eq!(Agda.extensions(), &["agda"]);
        assert_eq!(Agda.safe_mode(), Some("--safe --without-K"));
    }

    #[test]
    fn check_file_is_honest_about_availability() {
        // We cannot guarantee agda is absent in every environment, so we
        // assert the honesty invariant either way: absent ⇒ the graceful
        // `Unavailable` form (never an Err); present ⇒ the verdict is
        // strictly exit-code-derived, never fabricated.
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("X.agda");
        std::fs::write(&f, "module X where\n").unwrap();
        let out = Agda.check_file(&f, tmp.path()).unwrap();
        if out.available {
            assert!(matches!(out.verdict, Verdict::Proven | Verdict::Error));
            assert_eq!(out.ok, out.verdict == Verdict::Proven);
        } else {
            assert_eq!(out.verdict, Verdict::Unavailable);
            assert!(!out.ok);
            assert!(out.exit_code.is_none());
        }
    }
}
