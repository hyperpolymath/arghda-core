// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! The Agda backend — arghda's first (and v0.1 reference) [`Backend`] — plus
//! its Cubical-Agda sibling [`AgdaCubical`].
//!
//! Owns the `agda` shell-out; delegates import-graph parsing to the
//! Agda-specific free functions in [`crate::graph`] and the lint pack to
//! [`crate::lint`]. Assistant model: `agda -i <root> <file>`; exit 0 →
//! [`Verdict::Proven`], a run that errors → [`Verdict::Error`], an absent
//! binary → [`Verdict::Unavailable`]. Exit-code only — arghda never claims
//! a result Agda did not return. `Admitted`/`Postulated` are amber overlays
//! surfaced by the lint pack (they are not process facts), so `check_file`
//! deliberately does not manufacture them from a green exit.
//!
//! [`AgdaCubical`] is the same tool in `--cubical --safe` mode (NOT
//! `--without-K` — cubical is incompatible with it). It is a separate backend
//! rather than a mode flag on [`Agda`] because Cubical and `--safe --without-K`
//! Agda form disjoint *islands*: a file in one flag profile cannot import a
//! file in the other (Agda enforces this at typecheck; arghda surfaces the
//! resulting `Error` verdict honestly rather than pretending the edge is
//! sound). Everything except the invocation flag, the lint safe-pragma
//! profile, the display name and `safe_mode` is shared.

use super::{Backend, BackendKind, Outcome, Verdict};
use crate::graph;
use crate::lint::{agda_cubical_rules, rules_with_config, LintRule, RuleConfig};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

const TAIL_LINES: usize = 40;

/// The Agda proof assistant (`--safe --without-K` profile).
#[derive(Clone, Copy, Debug, Default)]
pub struct Agda;

/// Cubical Agda (`--cubical --safe` profile; a disjoint island from [`Agda`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct AgdaCubical;

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
        invoke_agda(file, include_root, false)
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

    fn command(&self) -> &'static str {
        "agda"
    }
}

impl Backend for AgdaCubical {
    fn name(&self) -> &'static str {
        "agda-cubical"
    }

    fn kind(&self) -> BackendKind {
        BackendKind::Assistant
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["agda"]
    }

    fn safe_mode(&self) -> Option<&'static str> {
        Some("--cubical --safe")
    }

    /// Typecheck with `--cubical` forced on the command line
    /// (`agda --cubical -i <include_root> <file>`).
    fn check_file(&self, file: &Path, include_root: &Path) -> Result<Outcome> {
        invoke_agda(file, include_root, true)
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
        agda_cubical_rules(cfg)
    }

    fn command(&self) -> &'static str {
        // Same binary as standard Agda; the mode is the `--cubical` flag.
        "agda"
    }
}

/// Shared `agda` invocation for both flag profiles. `cubical` adds the
/// `--cubical` command-line flag; otherwise the profile lives in the file's
/// own OPTIONS pragma. Exit 0 → Proven, ran-nonzero → Error, absent →
/// Unavailable.
fn invoke_agda(file: &Path, include_root: &Path, cubical: bool) -> Result<Outcome> {
    let mut cmd = Command::new("agda");
    if cubical {
        cmd.arg("--cubical");
    }
    let output = cmd.arg("-i").arg(include_root).arg(file).output();

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
                // Exit-code only: 0 is the sole signal Agda gives that means
                // "typechecked". Admitted/Postulated are lint-derived
                // overlays, not process facts.
                verdict: if ok { Verdict::Proven } else { Verdict::Error },
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(Outcome::unavailable(BackendKind::Assistant))
        }
        Err(e) => Err(e.into()),
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
    fn cubical_backend_identity() {
        assert_eq!(AgdaCubical.name(), "agda-cubical");
        assert_eq!(AgdaCubical.kind(), BackendKind::Assistant);
        assert_eq!(AgdaCubical.extensions(), &["agda"]);
        // The distinguishing fact: cubical is --safe but NOT --without-K.
        assert_eq!(AgdaCubical.safe_mode(), Some("--cubical --safe"));
    }

    #[test]
    fn command_and_probe_are_honest() {
        // Both Agda profiles run the same binary; the mode is a flag.
        assert_eq!(Agda.command(), "agda");
        assert_eq!(AgdaCubical.command(), "agda");
        // probe() reports availability honestly, whether or not agda is here.
        let p = Agda.probe();
        assert_eq!(p.backend, "agda");
        assert_eq!(p.kind, BackendKind::Assistant);
        assert!(!p.detail.is_empty());
        // agda-cubical probes the same binary, so agrees on runnability.
        assert_eq!(AgdaCubical.probe().runnable, p.runnable);
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
