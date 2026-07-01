// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell <j.d.a.jewell@open.ac.uk>

//! Cubical-Agda backend: the flag-profile island. Hermetic — asserts the
//! lint distinction (cubical files need `--cubical --safe`, not `--without-K`)
//! without invoking `agda`. The real typecheck is covered by the unit test in
//! `src/prover/agda.rs` and by manual dogfooding.

use arghda_core::lint::LintContext;
use arghda_core::{run_lints, Agda, AgdaCubical, Backend, RuleConfig, Severity};
use std::path::{Path, PathBuf};

fn cubical_all() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("cubical");
    p.push("All.agda");
    p
}

fn has_missing_safe_pragma(rule_owner: &dyn Backend, file: &Path) -> bool {
    let root = file.parent().unwrap();
    let roots = [file.to_path_buf()]; // file is its own root ⇒ orphan self-skips
    let ctx = LintContext {
        include_root: root,
        entry_modules: &roots,
    };
    let rules = rule_owner.lint_rules(&RuleConfig::default()).unwrap();
    let report = run_lints(file, &ctx, &rules).unwrap();
    report
        .diagnostics
        .iter()
        .any(|d| d.rule == "missing-safe-pragma" && d.severity == Severity::HardBlock)
}

#[test]
fn cubical_pack_accepts_a_cubical_file() {
    // The whole point of M2: `--cubical --safe` is a valid soundness profile.
    assert!(
        !has_missing_safe_pragma(&AgdaCubical, &cubical_all()),
        "the cubical lint pack must accept a --cubical --safe file"
    );
}

#[test]
fn standard_pack_flags_the_same_cubical_file() {
    // A cubical file lacks --without-K, so the standard Agda pack flags it —
    // which is exactly why the two profiles are disjoint islands with
    // separate packs.
    assert!(
        has_missing_safe_pragma(&Agda, &cubical_all()),
        "the standard pack must flag a cubical file (missing --without-K)"
    );
}

#[test]
fn cubical_backend_uses_agda_extension() {
    // Same extension as Agda: the island is a flag profile, not a file type.
    assert_eq!(AgdaCubical.extensions(), &["agda"]);
    assert_eq!(AgdaCubical.name(), "agda-cubical");
}
