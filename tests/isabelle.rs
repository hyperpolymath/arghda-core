// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Isabelle/HOL backend, exercised through the backend-parametric graph/dag/
//! reason path. These tests are hermetic — they only parse `.thy` text
//! (theory names, `imports` clauses) and the `ROOT` file, so they run with or
//! without the `isabelle` binary present. The real session build + verdict
//! classification (`check_file`) is covered by the unit test in
//! `src/prover/isabelle.rs` (honest either way) and by manual dogfooding
//! against Isabelle2025.

use arghda_core::lint::unpinned_headline::DEFAULT_HEADLINE_PATTERN;
use arghda_core::{build_dag, build_reason, Backend, Isabelle, LintRule};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn isabelle_fixture() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("isabelle");
    p
}

fn no_rules() -> Vec<Box<dyn LintRule>> {
    Vec::new()
}

#[test]
fn isabelle_imports_graph_over_the_fixture() {
    let root = isabelle_fixture();
    let roots = Isabelle.discover_roots(&root);
    let dag = build_dag(
        &root,
        &roots,
        &no_rules(),
        DEFAULT_HEADLINE_PATTERN,
        &Isabelle,
    )
    .unwrap();

    let mut ids: Vec<&str> = dag.nodes.iter().map(|n| n.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["All", "Base", "Orphan", "Util"]);

    // `imports` edges (Main is external and correctly dropped).
    assert!(dag
        .edges
        .iter()
        .any(|e| e.from == "All" && e.to == "Base" && e.kind == "imports"));
    assert!(dag.edges.iter().any(|e| e.from == "All" && e.to == "Util"));
    assert!(dag.edges.iter().any(|e| e.from == "Util" && e.to == "Base"));
    assert!(!dag.edges.iter().any(|e| e.to == "Orphan"));
    assert!(
        !dag.edges.iter().any(|e| e.to == "Main"),
        "stdlib import dropped"
    );
}

#[test]
fn isabelle_discover_roots_reads_the_root_file() {
    let root = isabelle_fixture();
    let roots = Isabelle.discover_roots(&root);
    let names: Vec<String> = roots
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    // The ROOT's `theories` section lists only `All`.
    assert_eq!(names, vec!["All.thy".to_string()]);
}

#[test]
fn isabelle_reason_graph_wires_the_root_cone() {
    let root = isabelle_fixture();
    let roots = Isabelle.discover_roots(&root);
    let dag = build_dag(
        &root,
        &roots,
        &no_rules(),
        DEFAULT_HEADLINE_PATTERN,
        &Isabelle,
    )
    .unwrap();
    let doc = build_reason(dag, &Isabelle, &BTreeMap::new(), &BTreeSet::new());

    assert_eq!(doc.crt_roots, vec!["All".to_string()]);
    let wired = |id: &str| doc.nodes.iter().find(|n| n.id == id).unwrap().wired;
    assert!(wired("All"));
    assert!(wired("Util"));
    assert!(wired("Base"));
    assert!(!wired("Orphan"), "Orphan must be unwired");
}
