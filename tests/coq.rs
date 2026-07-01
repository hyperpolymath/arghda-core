// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Coq / Rocq backend, exercised through the backend-parametric graph/dag/
//! reason path. These tests are hermetic — they only parse `.v` text (module
//! names + `Require` edges), so they run with or without the `coqc` binary
//! present. The real compile + verdict classification (`check_file`,
//! Section-aware postulate counting) is covered by the unit tests in
//! `src/prover/rocq.rs` (honest either way) and by manual dogfooding against
//! Coq 8.18.0.

use arghda_core::lint::unpinned_headline::DEFAULT_HEADLINE_PATTERN;
use arghda_core::{build_dag, build_reason, Backend, Coq, LintRule};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn coq_fixture() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("coq");
    p
}

/// The graph path needs no lint rules; a typed empty pack.
fn no_rules() -> Vec<Box<dyn LintRule>> {
    Vec::new()
}

#[test]
fn coq_require_graph_over_the_fixture() {
    let root = coq_fixture();
    let roots = Coq.discover_roots(&root);
    let dag = build_dag(&root, &roots, &no_rules(), DEFAULT_HEADLINE_PATTERN, &Coq).unwrap();

    let mut ids: Vec<&str> = dag.nodes.iter().map(|n| n.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["All", "Base", "Orphan", "Util"]);

    // `Require Import Base` / `Require Import Util` become edges.
    assert!(dag
        .edges
        .iter()
        .any(|e| e.from == "All" && e.to == "Base" && e.kind == "imports"));
    assert!(dag.edges.iter().any(|e| e.from == "All" && e.to == "Util"));
    assert!(dag.edges.iter().any(|e| e.from == "Util" && e.to == "Base"));
    // Nothing requires Orphan.
    assert!(!dag.edges.iter().any(|e| e.to == "Orphan"));
}

#[test]
fn coq_discover_roots_finds_all_v() {
    let root = coq_fixture();
    let roots = Coq.discover_roots(&root);
    let names: Vec<String> = roots
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["All.v".to_string()]);
}

#[test]
fn coq_reason_graph_wires_the_all_cone() {
    let root = coq_fixture();
    let roots = Coq.discover_roots(&root);
    let dag = build_dag(&root, &roots, &no_rules(), DEFAULT_HEADLINE_PATTERN, &Coq).unwrap();
    let doc = build_reason(dag, &Coq, &BTreeMap::new(), &BTreeSet::new());

    assert_eq!(doc.crt_roots, vec!["All".to_string()]);
    let wired = |id: &str| doc.nodes.iter().find(|n| n.id == id).unwrap().wired;
    assert!(wired("All"));
    assert!(wired("Util"));
    assert!(wired("Base"));
    assert!(!wired("Orphan"), "Orphan must be unwired");
}
