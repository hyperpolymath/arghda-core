// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! `reason` document construction over the fixtures — the Flying-Logic
//! reasoning graph built on top of the DAG.

use arghda_core::lint::unpinned_headline::DEFAULT_HEADLINE_PATTERN;
use arghda_core::{build_dag, build_reason, default_rules, Agda, Verdict};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    p
}

fn dag_of(name: &str) -> arghda_core::DagDocument {
    let root = fixture(name);
    let roots = [root.join("All.agda")];
    build_dag(
        &root,
        &roots,
        &default_rules(),
        DEFAULT_HEADLINE_PATTERN,
        &Agda,
    )
    .unwrap()
}

fn effective(doc: &arghda_core::ReasonDocument, id: &str) -> Verdict {
    doc.nodes.iter().find(|n| n.id == id).unwrap().effective
}

fn wired(doc: &arghda_core::ReasonDocument, id: &str) -> bool {
    doc.nodes.iter().find(|n| n.id == id).unwrap().wired
}

#[test]
fn no_verdicts_means_clean_nodes_are_unknown_not_proven() {
    // The honesty rule end-to-end: without a typecheck, a lint-clean tree is
    // all `unknown` — never fabricated green.
    let doc = build_reason(
        dag_of("wellformed"),
        &Agda,
        &BTreeMap::new(),
        &BTreeSet::new(),
    );
    for n in &doc.nodes {
        assert_eq!(
            n.effective,
            Verdict::Unknown,
            "node {} should be unknown without a typecheck",
            n.id
        );
        assert_eq!(n.soundness, "unproven");
    }
    assert_eq!(doc.crt_roots, vec!["All".to_string()]);
    assert_eq!(doc.version, "0.1");
    // The DAG is embedded verbatim.
    assert_eq!(doc.dag.version, "0.1");
}

#[test]
fn supplied_proven_lights_up_the_whole_cone() {
    let mut verdicts = BTreeMap::new();
    for id in ["All", "Good", "Util"] {
        verdicts.insert(id.to_string(), Verdict::Proven);
    }
    let doc = build_reason(dag_of("wellformed"), &Agda, &verdicts, &BTreeSet::new());
    for id in ["All", "Good", "Util"] {
        assert_eq!(
            effective(&doc, id),
            Verdict::Proven,
            "{id} should be proven"
        );
        assert!(wired(&doc, id), "{id} should be wired");
    }
}

#[test]
fn a_postulated_import_infects_its_downstream() {
    // The Flying-Logic payoff: Util is only Postulated (amber); All imports
    // Util (an And edge), so All's *effective* verdict is dragged down to
    // Postulated even though All itself typechecked. Good does not import
    // Util, so it stays Proven.
    let mut verdicts = BTreeMap::new();
    verdicts.insert("All".to_string(), Verdict::Proven);
    verdicts.insert("Good".to_string(), Verdict::Proven);
    verdicts.insert("Util".to_string(), Verdict::Postulated);
    let doc = build_reason(dag_of("wellformed"), &Agda, &verdicts, &BTreeSet::new());

    assert_eq!(effective(&doc, "Util"), Verdict::Postulated);
    assert_eq!(
        effective(&doc, "All"),
        Verdict::Postulated,
        "amber must propagate up the And import edge"
    );
    assert_eq!(effective(&doc, "Good"), Verdict::Proven);

    // Soundness buckets reflect it.
    let all = doc.nodes.iter().find(|n| n.id == "All").unwrap();
    assert_eq!(all.soundness, "amber");
}

#[test]
fn an_orphan_is_unwired() {
    // Orphan is reachable from no CI root: wired = false. All and Used are.
    let doc = build_reason(dag_of("orphan"), &Agda, &BTreeMap::new(), &BTreeSet::new());
    assert!(wired(&doc, "All"));
    assert!(wired(&doc, "Used"));
    assert!(!wired(&doc, "Orphan"), "Orphan must be unwired");
    assert_eq!(doc.crt_roots, vec!["All".to_string()]);
}

#[test]
fn staleness_demotes_a_proven_node_to_unknown() {
    let mut verdicts = BTreeMap::new();
    for id in ["All", "Good", "Util"] {
        verdicts.insert(id.to_string(), Verdict::Proven);
    }
    let mut stale = BTreeSet::new();
    stale.insert("Good".to_string());
    let doc = build_reason(dag_of("wellformed"), &Agda, &verdicts, &stale);
    assert_eq!(
        effective(&doc, "Good"),
        Verdict::Unknown,
        "a stale proven node must fall back to unknown"
    );
    // Util (not stale) stays proven; All imports Good so All is dragged to
    // unknown by the stale prerequisite.
    assert_eq!(effective(&doc, "Util"), Verdict::Proven);
    assert_eq!(effective(&doc, "All"), Verdict::Unknown);
}

#[test]
fn crt_roots_and_wiring_survive_discover_roots_relative_paths() {
    // Regression: the CLI derives roots via `graph::discover_roots`, which
    // yields include-root-PREFIXED relative paths (e.g.
    // `<fixture>/All.agda`), not absolute ones. crt_roots must still resolve
    // to the bare module id `All` and the cone must be wired — a bug here
    // left every node unwired. (dag_of passes absolute roots, where the
    // earlier `include_root.join` was a silent no-op, so it missed this.)
    let root = fixture("wellformed");
    let roots = arghda_core::graph::discover_roots(&root);
    let dag = build_dag(
        &root,
        &roots,
        &default_rules(),
        DEFAULT_HEADLINE_PATTERN,
        &Agda,
    )
    .unwrap();
    let doc = build_reason(dag, &Agda, &BTreeMap::new(), &BTreeSet::new());

    assert_eq!(
        doc.crt_roots,
        vec!["All".to_string()],
        "crt_roots must be the bare module id, got {:?}",
        doc.crt_roots
    );
    for id in ["All", "Good", "Util"] {
        assert!(
            wired(&doc, id),
            "{id} must be wired via discover_roots path"
        );
    }
}
