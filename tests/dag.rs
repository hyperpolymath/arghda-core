//! `dag` document construction over the fixtures.

use arghda_core::lint::unpinned_headline::DEFAULT_HEADLINE_PATTERN;
use arghda_core::{build_dag, default_rules};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    p
}

#[test]
fn dag_over_orphan_fixture_has_expected_shape() {
    let root = fixture("orphan");
    let roots = [root.join("All.agda")];
    let doc = build_dag(&root, &roots, &default_rules(), DEFAULT_HEADLINE_PATTERN).unwrap();

    // Nodes are deterministic and sorted by module id.
    let ids: Vec<&str> = doc.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, vec!["All", "Orphan", "Used"]);

    // The real import edge is present; nothing imports the orphan.
    assert!(doc
        .edges
        .iter()
        .any(|e| e.from == "All" && e.to == "Used" && e.kind == "imports"));
    assert!(!doc.edges.iter().any(|e| e.to == "Orphan"));

    // Orphan is blocked by the orphan-module rule; others are clean.
    let orphan = doc.nodes.iter().find(|n| n.id == "Orphan").unwrap();
    assert_eq!(orphan.status, "blocked");
    assert!(orphan
        .lint
        .hard_block
        .contains(&"orphan-module".to_string()));
    assert_eq!(orphan.file, PathBuf::from("Orphan.agda"));
    assert_eq!(
        doc.nodes.iter().find(|n| n.id == "Used").unwrap().status,
        "clean"
    );
    assert_eq!(
        doc.nodes.iter().find(|n| n.id == "All").unwrap().status,
        "clean"
    );

    // The blocked list names the orphan.
    assert!(doc.blocked.iter().any(|b| b.node == "Orphan"));
}

#[test]
fn dag_over_wellformed_fixture_is_all_clean() {
    let root = fixture("wellformed");
    let roots = [root.join("All.agda")];
    let doc = build_dag(&root, &roots, &default_rules(), DEFAULT_HEADLINE_PATTERN).unwrap();

    assert_eq!(doc.version, "0.1");
    assert!(
        doc.nodes.iter().all(|n| n.status == "clean"),
        "all nodes clean; got: {:?}",
        doc.nodes
    );
    assert!(doc.blocked.is_empty());
    assert!(doc.edges.iter().any(|e| e.from == "All" && e.to == "Good"));
    assert!(doc.edges.iter().any(|e| e.from == "All" && e.to == "Util"));
}

#[test]
fn dag_populates_node_headlines() {
    let root = fixture("headlines");
    let roots = [root.join("All.agda")];
    let doc = build_dag(&root, &roots, &default_rules(), DEFAULT_HEADLINE_PATTERN).unwrap();

    // `Thm` declares two top-level headline signatures (sorted, deduped); its
    // indented `private` helper is not top-level and is not surfaced.
    let thm = doc.nodes.iter().find(|n| n.id == "Thm").unwrap();
    assert_eq!(
        thm.headlines,
        vec!["thm-one".to_string(), "thm-two".to_string()]
    );

    // The entry module has only imports, so no headlines.
    let all = doc.nodes.iter().find(|n| n.id == "All").unwrap();
    assert!(all.headlines.is_empty());
}
