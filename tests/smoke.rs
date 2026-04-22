//! End-to-end smoke test: run lints over the echo-types Agda suite and
//! verify v0.1 expectations.
//!
//! Expectation: every `.agda` file in `proofs/agda` passes
//! `missing-safe-pragma` and `orphan-module` relative to `All.agda`.

use arghda_core::lint::{default_rules, LintContext};
use arghda_core::{run_lints, Severity};
use std::path::PathBuf;
use walkdir::WalkDir;

fn echo_types_root() -> PathBuf {
    // arghda-core sits at echo-types/arghda-core/.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p
}

#[test]
fn echo_types_passes_default_rules() {
    let root = echo_types_root();
    let include_root = root.join("proofs").join("agda");
    let entry = include_root.join("All.agda");

    assert!(entry.is_file(), "All.agda not found at {}", entry.display());

    let rules = default_rules();
    let ctx = LintContext {
        include_root: &include_root,
        entry_module: &entry,
    };

    let mut hard_blocks = Vec::new();

    for entry in WalkDir::new(&include_root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("agda") {
            continue;
        }
        let report = run_lints(path, &ctx, &rules).expect("lint run failed");
        for d in report.diagnostics {
            if d.severity == Severity::HardBlock {
                hard_blocks.push(format!("{}: {} — {}", path.display(), d.rule, d.message));
            }
        }
    }

    assert!(
        hard_blocks.is_empty(),
        "expected no hard-blocks, got:\n{}",
        hard_blocks.join("\n")
    );
}
