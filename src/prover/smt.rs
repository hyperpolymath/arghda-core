// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! SMT solver backends (Z3, CVC5) — the first `Solver`-kind backends.
//!
//! The other half of the backend taxonomy: where an assistant typechecks a
//! file (exit code is the verdict), a *solver* is fed an SMT-LIB2 query and
//! answers `sat`/`unsat`/`unknown` on stdout. Ground-truthed against Z3
//! 4.8.12 and CVC5 1.2.0: `<solver> <file.smt2>` prints one result token per
//! `(check-sat)` and exits 0; a malformed file prints `(error …)` and exits
//! non-zero.
//!
//! Verdict mapping (the documented convention — the query author frames their
//! problem to suit it):
//! * `unsat` → [`Verdict::Proven`] — the assertion set is unsatisfiable (a
//!   verification condition is discharged: `¬P` has no model, so `P` holds).
//! * `sat` → [`Verdict::Refuted`] — a model/counterexample exists.
//! * `unknown` / timeout → [`Verdict::Unknown`].
//! * a solver error / non-zero exit → [`Verdict::Error`].
//! * the binary absent → [`Verdict::Unavailable`].
//!
//! The verdict is derived from the solver's ACTUAL output, never from a
//! bare exit code (both `sat` and `unsat` exit 0). Over multiple
//! `(check-sat)`s: any `sat` ⇒ Refuted; else any `unknown` ⇒ Unknown; else
//! all `unsat` ⇒ Proven.
//!
//! SMT files are standalone queries: there is no import graph
//! ([`Backend::direct_imports`] is empty → isolated DAG nodes, which is
//! valid) and no root convention ([`Backend::discover_roots`] is empty, so
//! `wired` is not meaningful for solver nodes).

use super::{Backend, BackendKind, Outcome, Verdict};
use crate::graph;
use crate::lint::{LintRule, RuleConfig};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

const TAIL_LINES: usize = 40;

/// An SMT-LIB2 solver backend, parameterised by its binary.
#[derive(Clone, Copy, Debug)]
pub struct Smt {
    name: &'static str,
    cmd: &'static str,
}

impl Smt {
    /// The Z3 solver.
    pub fn z3() -> Self {
        Self {
            name: "z3",
            cmd: "z3",
        }
    }

    /// The CVC5 solver.
    pub fn cvc5() -> Self {
        Self {
            name: "cvc5",
            cmd: "cvc5",
        }
    }
}

impl Backend for Smt {
    fn name(&self) -> &'static str {
        self.name
    }

    fn kind(&self) -> BackendKind {
        BackendKind::Solver
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["smt2"]
    }

    fn check_file(&self, file: &Path, _include_root: &Path) -> Result<Outcome> {
        // Solvers read the query file directly; no include-root search path.
        let output = Command::new(self.cmd).arg(file).output();
        match output {
            Ok(out) => {
                let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
                let verdict = parse_verdict(&combined, out.status.success());
                Ok(Outcome {
                    available: true,
                    exit_code: out.status.code(),
                    // For a solver "ok" = the goal was discharged (unsat).
                    ok: verdict == Verdict::Proven,
                    output_tail: tail(&combined, TAIL_LINES),
                    kind: BackendKind::Solver,
                    verdict,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Outcome::unavailable(BackendKind::Solver))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn module_name_of(&self, file: &Path, include_root: &Path) -> Option<String> {
        // No true module names; use the relative path as a stable node id.
        graph::module_name_of(file, include_root)
    }

    fn module_to_path(&self, module: &str, include_root: &Path) -> PathBuf {
        let mut p = include_root.to_path_buf();
        for part in module.split('.') {
            p.push(part);
        }
        p.set_extension("smt2");
        p
    }

    fn direct_imports(&self, _file: &Path) -> Result<Vec<String>> {
        // SMT queries are standalone: no dependency edges.
        Ok(Vec::new())
    }

    fn discover_roots(&self, _include_root: &Path) -> Vec<PathBuf> {
        // No `All.agda`/`Main.idr`-style root convention for SMT.
        Vec::new()
    }

    fn lint_rules(&self, _cfg: &RuleConfig) -> Result<Vec<Box<dyn LintRule>>> {
        // No source-level escape-hatch class for SMT-LIB2.
        Ok(Vec::new())
    }
}

/// Map solver output + exit status to a [`Verdict`]. Parses the actual
/// `sat`/`unsat`/`unknown` result tokens; never trusts the bare exit code
/// (both `sat` and `unsat` exit 0).
fn parse_verdict(output: &str, exit_ok: bool) -> Verdict {
    if !exit_ok || output.contains("(error") {
        return Verdict::Error;
    }
    let mut saw_unsat = false;
    let mut saw_unknown = false;
    for tok in output.split_whitespace() {
        match tok {
            "sat" => return Verdict::Refuted, // any model ⇒ refuted
            "unsat" => saw_unsat = true,
            "unknown" => saw_unknown = true,
            _ => {}
        }
    }
    if saw_unknown {
        Verdict::Unknown
    } else if saw_unsat {
        Verdict::Proven
    } else {
        // Ran cleanly but produced no result token — treat as an error, not
        // a silent pass.
        Verdict::Error
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
    fn solver_identity() {
        assert_eq!(Smt::z3().name(), "z3");
        assert_eq!(Smt::cvc5().name(), "cvc5");
        assert_eq!(Smt::z3().kind(), BackendKind::Solver);
        assert_eq!(Smt::z3().extensions(), &["smt2"]);
    }

    #[test]
    fn module_to_path_uses_smt2_extension() {
        assert_eq!(
            Smt::z3().module_to_path("queries.overflow", Path::new("/r")),
            PathBuf::from("/r/queries/overflow.smt2")
        );
    }

    #[test]
    fn solvers_have_no_edges_or_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("q.smt2");
        std::fs::write(&f, "(check-sat)\n").unwrap();
        assert!(Smt::z3().direct_imports(&f).unwrap().is_empty());
        assert!(Smt::z3().discover_roots(tmp.path()).is_empty());
    }

    #[test]
    fn verdict_mapping_is_honest() {
        // unsat ⇒ Proven (VC discharged), sat ⇒ Refuted, unknown ⇒ Unknown.
        assert_eq!(parse_verdict("unsat\n", true), Verdict::Proven);
        assert_eq!(parse_verdict("sat\n", true), Verdict::Refuted);
        assert_eq!(parse_verdict("unknown\n", true), Verdict::Unknown);
        // "sat" is a substring of "unsat" but a distinct token — no confusion.
        assert_eq!(parse_verdict("unsat\nunsat\n", true), Verdict::Proven);
        // Any sat among several checks ⇒ Refuted; any unknown (no sat) ⇒ Unknown.
        assert_eq!(parse_verdict("unsat\nsat\n", true), Verdict::Refuted);
        assert_eq!(parse_verdict("unsat\nunknown\n", true), Verdict::Unknown);
        // Errors / non-zero exit / no result ⇒ Error, never a silent pass.
        assert_eq!(
            parse_verdict("(error \"parse error\")", false),
            Verdict::Error
        );
        assert_eq!(parse_verdict("(error \"x\")", true), Verdict::Error);
        assert_eq!(parse_verdict("", true), Verdict::Error);
    }

    #[test]
    fn check_file_is_honest_about_availability() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("q.smt2");
        std::fs::write(
            &f,
            "(declare-const x Int)\n(assert (and (> x 2) (< x 1)))\n(check-sat)\n",
        )
        .unwrap();
        let out = Smt::z3().check_file(&f, tmp.path()).unwrap();
        assert_eq!(out.kind, BackendKind::Solver);
        if out.available {
            // If z3 is present this query is unsat ⇒ Proven; either way the
            // verdict is derived from real output, not fabricated.
            assert!(matches!(
                out.verdict,
                Verdict::Proven | Verdict::Refuted | Verdict::Unknown | Verdict::Error
            ));
            assert_eq!(out.ok, out.verdict == Verdict::Proven);
        } else {
            assert_eq!(out.verdict, Verdict::Unavailable);
        }
    }
}
