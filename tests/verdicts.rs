// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell <j.d.a.jewell@open.ac.uk>

//! Known-answer verdict matrix: every backend's `check_file` run against the
//! REAL tool, on fixtures whose correct verdict is fixed in advance.
//!
//! The unit tests in `src/prover/*.rs` only show that `check_file` never
//! fabricates a pass (they accept `Proven | Error | Unavailable`), so they
//! cannot catch a backend that maps a real result to the wrong verdict. This
//! file asserts the exact verdict for each case:
//!
//! * `proven-*`     — a sound artefact the tool accepts.
//! * `admitted-*`   — a hole (`sorry`, `Admitted`, `?name`).
//! * `postulated-*` — an axiom / postulate.
//! * `error-*`      — a type or parse error.
//! * `refuted-*`    — a satisfiable SMT query (a model exists).
//!
//! Gating — fail-closed, never a silent green. These tests are `#[ignore]`d,
//! so a plain `cargo test` reports them as *ignored* (not run, and says so)
//! rather than passing in an environment without the tools. Run them with
//! `just verdicts` (`cargo test --test verdicts -- --ignored`). In that run a
//! missing tool is a FAILURE, unless its backend is named in
//! `ARGHDA_ALLOW_MISSING` (comma-separated backend names, e.g.
//! `isabelle,idris2`); an allowed miss is printed as `SKIP`.
//!
//! Each case is copied to a fresh temp directory first, so tools that write
//! build products beside the source (Agda's `.agdai`) never touch the tree.

use arghda_core::lint::LintContext;
use arghda_core::{
    run_lints, Agda, AgdaCubical, Backend, Coq, Idris2, Isabelle, Lean, Mizar, RuleConfig, Smt,
    Verdict,
};
use std::path::{Path, PathBuf};

fn case_dir(backend: &str, case: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.extend(["tests", "fixtures", "verdicts", backend, case]);
    p
}

/// The single source file in a case directory, copied into `into`.
fn stage(src_dir: &Path, into: &Path) -> PathBuf {
    let mut files: Vec<PathBuf> = std::fs::read_dir(src_dir)
        .unwrap_or_else(|e| panic!("fixture dir {}: {e}", src_dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    assert_eq!(
        files.len(),
        1,
        "one source file per case: {}",
        src_dir.display()
    );
    let src = files.pop().unwrap();
    let dst = into.join(src.file_name().unwrap());
    std::fs::copy(&src, &dst).unwrap();
    dst
}

fn allowed_missing(backend: &str) -> bool {
    std::env::var("ARGHDA_ALLOW_MISSING")
        .map(|v| v.split(',').any(|b| b.trim() == backend))
        .unwrap_or(false)
}

/// Check one case. Returns `false` (and prints SKIP) only when the tool is
/// absent AND its backend is named in `ARGHDA_ALLOW_MISSING`; a missing tool
/// otherwise fails, and so does any wrong verdict.
fn check_case(backend: &dyn Backend, case: &str, expected: Verdict) -> bool {
    let name = backend.name();
    let tmp = tempfile::tempdir().unwrap();
    let file = stage(&case_dir(name, case), tmp.path());
    let out = backend.check_file(&file, tmp.path()).unwrap();
    if out.verdict == Verdict::Unavailable {
        assert!(
            allowed_missing(name),
            "{name}/{case}: tool unavailable (install it, or name `{name}` in \
             ARGHDA_ALLOW_MISSING to skip it explicitly)"
        );
        eprintln!("SKIP (tool unavailable): {name}/{case}");
        return false;
    }
    assert_eq!(
        out.verdict, expected,
        "{name}/{case}: expected {expected:?}, got {:?}\n--- tool output ---\n{}",
        out.verdict, out.output_tail
    );
    assert_eq!(
        out.ok,
        expected == Verdict::Proven,
        "{name}/{case}: `ok` must mean Proven"
    );
    true
}

fn run_matrix(backend: &dyn Backend, cases: &[(&str, Verdict)]) {
    for (case, expected) in cases {
        check_case(backend, case, *expected);
    }
}

#[test]
#[ignore = "needs the real provers; run with `just verdicts`"]
fn agda_verdicts() {
    run_matrix(
        &Agda,
        &[
            ("proven-good", Verdict::Proven),
            ("error-broken", Verdict::Error),
        ],
    );
}

/// The lint rules that fire on a staged copy of `backend/case`, with the
/// file as its own root so `orphan-module` self-skips.
fn lint_rules_fired(backend: &dyn Backend, case: &str) -> Vec<String> {
    let tmp = tempfile::tempdir().unwrap();
    let file = stage(&case_dir(backend.name(), case), tmp.path());
    let roots = [file.clone()];
    let ctx = LintContext {
        include_root: tmp.path(),
        entry_modules: &roots,
    };
    let pack = backend.lint_rules(&RuleConfig::default()).unwrap();
    let report = run_lints(&file, &ctx, &pack).unwrap();
    report.diagnostics.iter().map(|d| d.rule.clone()).collect()
}

/// Agda's backend verdict is exit-code only by design (see `src/prover/agda.rs`):
/// a non-`--safe` file with a postulate typechecks, so the backend says
/// `Proven`. The postulate is caught by the lint pack as two hard blocks, so
/// `arghda check` reports `rejected`. NOTE: as of this commit `arghda promote`
/// does not consult the lint pack, so the hard block is reported but not
/// enforced on the `working -> proven` move (spec §lint says it forbids it).
/// This test pins what the backend and lint pack do, not promotion.
#[test]
#[ignore = "needs the real provers; run with `just verdicts`"]
fn agda_postulate_is_green_to_the_tool_but_hard_blocked_by_lint() {
    if !check_case(&Agda, "postulated-post", Verdict::Proven) {
        return;
    }
    let rules = lint_rules_fired(&Agda, "postulated-post");
    for want in ["unjustified-postulate", "missing-safe-pragma"] {
        assert!(
            rules.iter().any(|r| r == want),
            "lint rules fired: {rules:?}"
        );
    }
}

#[test]
#[ignore = "needs the real provers; run with `just verdicts`"]
fn agda_cubical_verdicts() {
    run_matrix(&AgdaCubical, &[("proven-good", Verdict::Proven)]);
}

#[test]
#[ignore = "needs the real provers; run with `just verdicts`"]
fn idris2_verdicts() {
    run_matrix(
        &Idris2,
        &[
            ("proven-good", Verdict::Proven),
            ("error-broken", Verdict::Error),
        ],
    );
}

/// Like Agda, the Idris2 backend is exit-code only (`src/prover/idris2.rs`):
/// `idris2 --check` accepts a file with an unsolved hole, so the backend says
/// `Proven`, and the hole is surfaced only by the `escape-hatch` lint. That
/// lint is a WARN, so `arghda check` reports such a file as
/// `proven-eligible`; only the reason graph caps it at `Admitted`
/// (`reason::lint_cap`). This test pins the current facts; it does not
/// claim the `check` verdict is right for a holed file.
#[test]
#[ignore = "needs the real provers; run with `just verdicts`"]
fn idris2_hole_is_green_to_the_tool_and_only_warned_by_lint() {
    if !check_case(&Idris2, "admitted-hole", Verdict::Proven) {
        return;
    }
    let rules = lint_rules_fired(&Idris2, "admitted-hole");
    assert!(
        rules.iter().any(|r| r == "escape-hatch"),
        "lint rules fired: {rules:?}"
    );
}

#[test]
#[ignore = "needs the real provers; run with `just verdicts`"]
fn lean4_verdicts() {
    run_matrix(
        &Lean,
        &[
            ("proven-good", Verdict::Proven),
            ("admitted-sorry", Verdict::Admitted),
            ("postulated-axiom", Verdict::Postulated),
            ("error-broken", Verdict::Error),
        ],
    );
}

#[test]
#[ignore = "needs the real provers; run with `just verdicts`"]
fn coq_verdicts() {
    run_matrix(
        &Coq,
        &[
            ("proven-good", Verdict::Proven),
            ("admitted-admitted", Verdict::Admitted),
            ("postulated-axiom", Verdict::Postulated),
            ("error-broken", Verdict::Error),
        ],
    );
}

#[test]
#[ignore = "needs the real provers; run with `just verdicts`"]
fn isabelle_verdicts() {
    run_matrix(
        &Isabelle,
        &[
            ("proven-good", Verdict::Proven),
            ("admitted-sorry", Verdict::Admitted),
            ("postulated-axiom", Verdict::Postulated),
            ("error-broken", Verdict::Error),
        ],
    );
}

#[test]
#[ignore = "needs the real provers; run with `just verdicts`"]
fn mizar_verdicts() {
    run_matrix(
        &Mizar,
        &[
            ("proven-good", Verdict::Proven),
            ("error-broken", Verdict::Error),
        ],
    );
}

fn smt_cases() -> [(&'static str, Verdict); 3] {
    [
        ("proven-unsat", Verdict::Proven),
        ("refuted-sat", Verdict::Refuted),
        ("error-broken", Verdict::Error),
    ]
}

#[test]
#[ignore = "needs the real provers; run with `just verdicts`"]
fn z3_verdicts() {
    run_matrix(&Smt::z3(), &smt_cases());
}

#[test]
#[ignore = "needs the real provers; run with `just verdicts`"]
fn cvc5_verdicts() {
    run_matrix(&Smt::cvc5(), &smt_cases());
}
