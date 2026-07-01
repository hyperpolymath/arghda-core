// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Idris2 backend, exercised through the backend-parametric graph/dag/reason
//! path. These tests are hermetic — they only parse `.idr` text (module
//! names + imports), so they run with or without the `idris2` binary present.
//! The real typecheck (`check_file`) is covered by the unit test in
//! `src/prover/idris2.rs` (honest either way) and by manual dogfooding.

use arghda_core::lint::unpinned_headline::DEFAULT_HEADLINE_PATTERN;
use arghda_core::{build_dag, build_reason, Backend, Idris2, LintRule};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn idris2_fixture() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("idris2");
    p
}

/// The graph path needs no lint rules; a typed empty pack.
fn no_rules() -> Vec<Box<dyn LintRule>> {
    Vec::new()
}

#[test]
fn idris2_import_graph_over_the_fixture() {
    let root = idris2_fixture();
    // No lint rules needed to exercise the graph; the Idris2 backend drives
    // extension filtering (.idr), module naming, and import parsing.
    let roots = Idris2.discover_roots(&root);
    let dag = build_dag(
        &root,
        &roots,
        &no_rules(),
        DEFAULT_HEADLINE_PATTERN,
        &Idris2,
    )
    .unwrap();

    let ids: Vec<&str> = dag.nodes.iter().map(|n| n.id.as_str()).collect();
    // Dotted Idris2 module names, sorted.
    assert_eq!(ids, vec!["Data.Helper", "Main", "Orphan", "Util"]);

    // `import Util` and `import public Data.Helper` both become edges.
    assert!(dag
        .edges
        .iter()
        .any(|e| e.from == "Main" && e.to == "Util" && e.kind == "imports"));
    assert!(dag
        .edges
        .iter()
        .any(|e| e.from == "Main" && e.to == "Data.Helper"));
    // Nothing imports Orphan.
    assert!(!dag.edges.iter().any(|e| e.to == "Orphan"));
}

#[test]
fn idris2_discover_roots_finds_main() {
    let root = idris2_fixture();
    let roots = Idris2.discover_roots(&root);
    let names: Vec<String> = roots
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["Main.idr".to_string()]);
}

#[test]
fn idris2_reason_graph_wires_the_main_cone() {
    let root = idris2_fixture();
    let roots = Idris2.discover_roots(&root);
    let dag = build_dag(
        &root,
        &roots,
        &no_rules(),
        DEFAULT_HEADLINE_PATTERN,
        &Idris2,
    )
    .unwrap();
    let doc = build_reason(dag, &Idris2, &BTreeMap::new(), &BTreeSet::new());

    assert_eq!(doc.crt_roots, vec!["Main".to_string()]);
    let wired = |id: &str| doc.nodes.iter().find(|n| n.id == id).unwrap().wired;
    assert!(wired("Main"));
    assert!(wired("Util"));
    assert!(wired("Data.Helper"));
    assert!(!wired("Orphan"), "Orphan must be unwired");
}
