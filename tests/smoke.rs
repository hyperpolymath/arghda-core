//! End-to-end smoke tests for the v0.1 lint pack.
//!
//! Each test runs the default rule pack against a self-contained
//! fixture directory under `tests/fixtures/`. The fixtures stand in
//! for a real Agda workspace; they are not type-checked (arghda only
//! reads them as text). When arghda lived inside echo-types the smoke
//! test ran against echo-types' real `proofs/agda/All.agda`; after the
//! 2026-05-30 extraction the integration target became these
//! deliberately-tiny fixtures so the test stays self-contained.

use arghda_core::lint::{default_rules, LintContext};
use arghda_core::{run_lints, Severity};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    p
}

/// Run the default rule pack against every `.agda` file under
/// `include_root`, partitioning hits by rule name. Each hit becomes
/// `(file_basename, rule_name)`.
fn collect_hard_blocks(include_root: &Path) -> Vec<(String, String)> {
    let entry = include_root.join("All.agda");
    assert!(
        entry.is_file(),
        "fixture invariant: {} must contain All.agda",
        include_root.display()
    );

    let roots = [entry];
    let rules = default_rules();
    let ctx = LintContext {
        include_root,
        entry_modules: &roots,
    };

    let mut hits = Vec::new();
    for entry in WalkDir::new(include_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("agda") {
            continue;
        }
        let report = run_lints(path, &ctx, &rules).expect("lint run failed");
        for d in report.diagnostics {
            if d.severity == Severity::HardBlock {
                let basename = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string();
                hits.push((basename, d.rule));
            }
        }
    }
    hits
}

#[test]
fn wellformed_fixture_passes_both_default_rules() {
    let hits = collect_hard_blocks(&fixture("wellformed"));
    assert!(
        hits.is_empty(),
        "wellformed fixture must produce no hard-blocks, got: {:?}",
        hits
    );
}

#[test]
fn orphan_module_flagged_when_unreachable_from_entry() {
    let hits = collect_hard_blocks(&fixture("orphan"));
    let orphan_hits: Vec<_> = hits.iter().filter(|(_f, r)| r == "orphan-module").collect();
    assert!(
        orphan_hits.iter().any(|(f, _)| f == "Orphan.agda"),
        "orphan-module rule must flag Orphan.agda; hits were: {:?}",
        hits
    );
    assert!(
        !orphan_hits
            .iter()
            .any(|(f, _)| f == "Used.agda" || f == "All.agda"),
        "orphan-module rule must NOT flag reachable files; hits were: {:?}",
        hits
    );
}

#[test]
fn missing_safe_pragma_flagged_when_pragma_absent() {
    let hits = collect_hard_blocks(&fixture("missing_pragma"));
    let pragma_hits: Vec<_> = hits
        .iter()
        .filter(|(_f, r)| r == "missing-safe-pragma")
        .collect();
    assert!(
        pragma_hits.iter().any(|(f, _)| f == "Bad.agda"),
        "missing-safe-pragma rule must flag Bad.agda; hits were: {:?}",
        hits
    );
    assert!(
        !pragma_hits.iter().any(|(f, _)| f == "All.agda"),
        "missing-safe-pragma rule must NOT flag well-formed All.agda; hits were: {:?}",
        hits
    );
}
