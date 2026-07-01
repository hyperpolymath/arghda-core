// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell <j.d.a.jewell@open.ac.uk>

//! Lean4 backend, exercised through the backend-parametric graph/dag/reason
//! path. Hermetic — parses `.lean` text only, so it runs with or without the
//! `lean` binary. The real elaboration + honest-verdict path (`check_file`)
//! is covered by the unit tests in `src/prover/lean.rs` and by dogfooding.

use arghda_core::lint::unpinned_headline::DEFAULT_HEADLINE_PATTERN;
use arghda_core::{build_dag, build_reason, Backend, Lean, LintRule};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn lean_fixture() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("lean");
    p
}

fn no_rules() -> Vec<Box<dyn LintRule>> {
    Vec::new()
}

#[test]
fn lean_import_graph_and_root() {
    let root = lean_fixture();
    let roots = Lean.discover_roots(&root);
    let names: Vec<String> = roots
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["Main.lean".to_string()]);

    let dag = build_dag(&root, &roots, &no_rules(), DEFAULT_HEADLINE_PATTERN, &Lean).unwrap();
    let mut ids: Vec<&str> = dag.nodes.iter().map(|n| n.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["Main", "Util"]);
    // `import Util` becomes an edge; `open` (none here) would not.
    assert!(dag
        .edges
        .iter()
        .any(|e| e.from == "Main" && e.to == "Util" && e.kind == "imports"));
}

#[test]
fn lean_reason_graph_wires_the_main_cone() {
    let root = lean_fixture();
    let roots = Lean.discover_roots(&root);
    let dag = build_dag(&root, &roots, &no_rules(), DEFAULT_HEADLINE_PATTERN, &Lean).unwrap();
    let doc = build_reason(dag, &Lean, &BTreeMap::new(), &BTreeSet::new());

    assert_eq!(doc.crt_roots, vec!["Main".to_string()]);
    let wired = |id: &str| doc.nodes.iter().find(|n| n.id == id).unwrap().wired;
    assert!(wired("Main"));
    assert!(wired("Util"));
}
