// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Mizar backend, exercised through the backend-parametric graph/dag/reason
//! path. These tests are hermetic — they only parse `.miz` text (the `environ`
//! block's directives), so they run with or without the `verifier`/`accom`
//! binaries and `MIZFILES`. The real verify (`check_file` → accom + verifier +
//! `.err`) is covered by the unit test in `src/prover/mizar.rs` (honest either
//! way) and by manual dogfooding against Mizar 8.1.15.

use arghda_core::lint::unpinned_headline::DEFAULT_HEADLINE_PATTERN;
use arghda_core::{build_dag, build_reason, Backend, LintRule, Mizar};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn mizar_fixture() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("mizar");
    p
}

fn no_rules() -> Vec<Box<dyn LintRule>> {
    Vec::new()
}

#[test]
fn mizar_environ_graph_over_the_fixture() {
    let root = mizar_fixture();
    let roots = Mizar.discover_roots(&root);
    let dag = build_dag(&root, &roots, &no_rules(), DEFAULT_HEADLINE_PATTERN, &Mizar).unwrap();

    let mut ids: Vec<&str> = dag.nodes.iter().map(|n| n.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["base", "orphan", "user"]);

    // `user`'s environ references `BASE` (in-tree, lower-cased) → an edge; the
    // MML reference `XBOOLE_0` is external and dropped.
    assert!(dag
        .edges
        .iter()
        .any(|e| e.from == "user" && e.to == "base" && e.kind == "imports"));
    assert!(
        !dag.edges.iter().any(|e| e.to == "xboole_0"),
        "MML import dropped as external"
    );
    assert!(!dag.edges.iter().any(|e| e.to == "orphan"));
}

#[test]
fn mizar_has_no_root_convention() {
    // Mizar has no aggregator / entry-article convention — an empty root set is
    // honest, so nothing is "wired" (unlike Agda's All.agda cone).
    let root = mizar_fixture();
    assert!(Mizar.discover_roots(&root).is_empty());

    let dag = build_dag(&root, &[], &no_rules(), DEFAULT_HEADLINE_PATTERN, &Mizar).unwrap();
    let doc = build_reason(dag, &Mizar, &BTreeMap::new(), &BTreeSet::new());
    assert!(doc.crt_roots.is_empty());
    assert!(doc.nodes.iter().all(|n| !n.wired));
}
