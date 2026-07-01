// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell <j.d.a.jewell@open.ac.uk>

//! SMT solver backend (Z3/CVC5), exercised through the backend-parametric
//! graph/dag/reason path. Hermetic — the graph path never runs the solver, so
//! these pass with or without `z3`/`cvc5` present. The real solve path
//! (`check_file` + verdict parsing) is covered by the unit tests in
//! `src/prover/smt.rs` and by manual dogfooding.

use arghda_core::lint::unpinned_headline::DEFAULT_HEADLINE_PATTERN;
use arghda_core::{build_dag, build_reason, Backend, LintRule, Smt};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn smt_fixture() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("smt");
    p
}

fn no_rules() -> Vec<Box<dyn LintRule>> {
    Vec::new()
}

#[test]
fn smt_files_are_isolated_nodes_with_no_edges() {
    let root = smt_fixture();
    let z3 = Smt::z3();
    // Solvers have no root convention.
    assert!(z3.discover_roots(&root).is_empty());

    let dag = build_dag(&root, &[], &no_rules(), DEFAULT_HEADLINE_PATTERN, &z3).unwrap();
    let mut ids: Vec<&str> = dag.nodes.iter().map(|n| n.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["sat", "unsat"]);
    assert!(dag.edges.is_empty(), "SMT queries have no dependency edges");
}

#[test]
fn smt_reason_graph_has_nodes_but_no_crt_cone() {
    let root = smt_fixture();
    let z3 = Smt::z3();
    let dag = build_dag(&root, &[], &no_rules(), DEFAULT_HEADLINE_PATTERN, &z3).unwrap();
    let doc = build_reason(dag, &z3, &BTreeMap::new(), &BTreeSet::new());

    assert_eq!(doc.nodes.len(), 2);
    // No root convention ⇒ no CRT roots ⇒ nothing is "wired" (honest: a
    // standalone SMT query is not part of a verified-suite cone).
    assert!(doc.crt_roots.is_empty());
    assert!(doc.nodes.iter().all(|n| !n.wired));
}
