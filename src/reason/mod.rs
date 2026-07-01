// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! The Flying-Logic reasoning graph — arghda's status-propagating layer.
//!
//! This *wraps*, it does not replace, [`crate::dag::DagDocument`]: the DAG
//! (files, lint, headlines, import edges) is embedded verbatim as
//! [`ReasonDocument::dag`], and this module adds the reasoning semantics on
//! top — a per-node [`Verdict`], `And`/`Or` juncts on edges, and a
//! propagation fold that turns the import DAG into a live picture of what is
//! *actually* established.
//!
//! ## The honesty invariant (owner directive)
//!
//! Propagation is **demote-only**: a node's effective verdict can only be
//! *lowered* by its dependencies, never manufactured. Being lint-clean does
//! NOT make a node [`Verdict::Proven`] — only a real prover exit-0 (supplied
//! via the `verdicts` map) does. An unchecked-but-clean node is
//! [`Verdict::Unknown`], honestly. `Admitted`/`Postulated` are amber, never
//! green. This is the same rule the whole tool lives by: never report a
//! result the tool did not return.
//!
//! ## What propagates even without a typecheck
//!
//! Structural facts read straight from source *do* propagate. A module the
//! linter flags with `unjustified-postulate` is capped at `Postulated`
//! (amber); a `missing-safe-pragma` module is capped at `Admitted`; a
//! `believe_me`/`trustMe` escape hatch caps at `Admitted`. And because the
//! fold is demote-only over `And`-edges, that amber **infects downstream**:
//! anything that transitively imports a postulated module is itself capped
//! at `Postulated`. That is the Flying-Logic payoff — visible before a
//! single prover is run.

use crate::dag::DagDocument;
use crate::prover::{Backend, Verdict};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// How a node's dependencies combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Junct {
    /// All dependencies are required (the import case): the node is demoted
    /// to the *worst* of its `And`-dependencies.
    And,
    /// Alternative routes to one goal: the node is demoted only to the
    /// *best* alternative — one green route keeps it up.
    Or,
}

/// A reasoning edge: a dependency of `from` on `to`, with its junct.
#[derive(Clone, Debug, Serialize)]
pub struct ReasonEdge {
    pub from: String,
    pub to: String,
    pub junct: Junct,
    /// The underlying relation (carried from the import graph: `imports`).
    pub kind: String,
}

/// The reasoning overlay for one node.
#[derive(Clone, Debug, Serialize)]
pub struct ReasonNode {
    pub id: String,
    /// The node's own verdict, before dependency propagation — the supplied
    /// prover verdict (if any) demoted by this node's own lint caps.
    pub self_verdict: Verdict,
    /// The verdict after the demote-only fold over dependencies.
    pub effective: Verdict,
    /// Why `self_verdict` is what it is (supplied verdict + lint caps + any
    /// staleness), human-readable.
    pub verdict_evidence: String,
    /// Coarse classification of `effective`: `sound` | `amber` | `unproven`
    /// | `unsound`.
    pub soundness: &'static str,
    /// Whether this node is reachable from a CRT root (part of the verified
    /// suite). `false` = an orphan; "unwired is not done".
    pub wired: bool,
}

/// The full reasoning document: the DAG plus the reasoning overlay.
#[derive(Clone, Debug, Serialize)]
pub struct ReasonDocument {
    /// Reasoning-schema version (distinct from the embedded `dag.version`).
    pub version: &'static str,
    /// The import/lint DAG, embedded verbatim so existing consumers keep
    /// working.
    pub dag: DagDocument,
    pub nodes: Vec<ReasonNode>,
    pub edges: Vec<ReasonEdge>,
    /// The Current Reality Tree roots: the CI entry modules' ids. The
    /// "Proven cone" is the set of `wired` nodes whose `effective` is
    /// `Proven`, reachable from these.
    pub crt_roots: Vec<String>,
}

/// Greenness rank: higher is better. The demote-fold keeps the lower rank.
/// `Proven` is the sole green; `Admitted`/`Postulated` are amber;
/// `Unknown`/`Unavailable` are not-yet-established; `Refuted`/`Error` are
/// the floor.
fn rank(v: Verdict) -> u8 {
    match v {
        Verdict::Proven => 6,
        Verdict::Admitted => 5,
        Verdict::Postulated => 4,
        Verdict::Unknown => 3,
        Verdict::Unavailable => 2,
        Verdict::Refuted => 1,
        Verdict::Error => 0,
    }
}

/// Keep the worse (lower-rank) of two verdicts — the demote operator.
fn demote(a: Verdict, b: Verdict) -> Verdict {
    if rank(a) <= rank(b) {
        a
    } else {
        b
    }
}

/// Keep the better (higher-rank) of two verdicts — used for `Or` groups.
fn promote(a: Verdict, b: Verdict) -> Verdict {
    if rank(a) >= rank(b) {
        a
    } else {
        b
    }
}

/// Coarse soundness bucket for display.
fn soundness_of(v: Verdict) -> &'static str {
    match v {
        Verdict::Proven => "sound",
        Verdict::Admitted | Verdict::Postulated => "amber",
        Verdict::Unknown | Verdict::Unavailable => "unproven",
        Verdict::Refuted | Verdict::Error => "unsound",
    }
}

/// The amber/error cap a single lint rule imposes on a node's self-verdict,
/// if any. Structural facts read from source: a postulate cannot be more
/// than `Postulated`; a non-`--safe` module or an escape hatch cannot be
/// more than `Admitted`. `orphan-module` is deliberately absent — it affects
/// `wired`, not soundness (an orphan may typecheck perfectly). Rules with no
/// soundness meaning return `None`.
fn lint_cap(rule: &str) -> Option<Verdict> {
    match rule {
        "unjustified-postulate" => Some(Verdict::Postulated),
        "missing-safe-pragma" => Some(Verdict::Admitted),
        "escape-hatch" => Some(Verdict::Admitted),
        _ => None,
    }
}

/// Derive a node's self-verdict and the evidence string: start from the
/// supplied prover verdict (or `Unknown` if none was run), then demote by
/// every lint cap this node's own diagnostics impose.
fn self_verdict_of(
    id: &str,
    hard_block: &[String],
    warn: &[String],
    verdicts: &BTreeMap<String, Verdict>,
) -> (Verdict, String) {
    let mut evidence = Vec::new();
    let mut v = match verdicts.get(id) {
        Some(&supplied) => {
            evidence.push(format!("prover: {}", verdict_word(supplied)));
            supplied
        }
        None => {
            evidence.push("no typecheck run".to_string());
            Verdict::Unknown
        }
    };
    for rule in hard_block.iter().chain(warn.iter()) {
        if let Some(cap) = lint_cap(rule) {
            let before = v;
            v = demote(v, cap);
            if rank(v) < rank(before) {
                evidence.push(format!("{rule} → {}", verdict_word(cap)));
            }
        }
    }
    (v, evidence.join("; "))
}

fn verdict_word(v: Verdict) -> &'static str {
    match v {
        Verdict::Proven => "proven",
        Verdict::Refuted => "refuted",
        Verdict::Unknown => "unknown",
        Verdict::Admitted => "admitted",
        Verdict::Postulated => "postulated",
        Verdict::Error => "error",
        Verdict::Unavailable => "unavailable",
    }
}

/// DFS colour for cycle-safe propagation.
#[derive(Clone, Copy, PartialEq)]
enum Colour {
    White,
    Grey,
    Black,
}

/// Build the reasoning document from an already-built [`DagDocument`].
///
/// `verdicts` supplies real prover results by module id (from a `check` run
/// or a workspace's `proven/` state); absent ids are `Unknown`. `stale` is
/// the set of ids whose content/closure hash changed since they were proven
/// — a stale `Proven` is demoted to `Unknown`. `backend` maps the DAG's
/// entry-module paths to ids for the CRT roots.
pub fn build(
    dag: DagDocument,
    backend: &dyn Backend,
    verdicts: &BTreeMap<String, Verdict>,
    stale: &BTreeSet<String>,
) -> ReasonDocument {
    // CRT roots: the entry modules' ids. `entry_modules` are already full
    // paths (from `discover_roots`, which yields include-root-prefixed paths,
    // or from `--entry` as given), so they go straight to `module_name_of`
    // WITHOUT re-joining include_root — the same convention the orphan-module
    // rule uses. (Re-joining double-prefixes a relative entry and breaks the
    // id match, leaving every node unwired.)
    let crt_roots: Vec<String> = dag
        .entry_modules
        .iter()
        .filter_map(|p| backend.module_name_of(p, &dag.include_root))
        .collect();

    // Import edges are all conjunctive (you cannot typecheck a module unless
    // every import does). `Or` is supported by the model + fold for authored
    // graphs, but is never synthesised from imports.
    let edges: Vec<ReasonEdge> = dag
        .edges
        .iter()
        .map(|e| ReasonEdge {
            from: e.from.clone(),
            to: e.to.clone(),
            junct: Junct::And,
            kind: e.kind.to_string(),
        })
        .collect();

    // Adjacency: id -> [(dep_id, junct)].
    let mut adj: BTreeMap<String, Vec<(String, Junct)>> = BTreeMap::new();
    for e in &edges {
        adj.entry(e.from.clone())
            .or_default()
            .push((e.to.clone(), e.junct));
    }

    // wired = reachable from any CRT root by following edges.
    let wired = reachable(&crt_roots, &adj);

    // Self-verdict per node from its lint summary + supplied verdicts, then
    // staleness. A stale proof (content/closure hash changed since it was
    // proven) is no longer established: drop it to `Unknown` *before* the
    // fold, so the staleness propagates to everything that imports it.
    let mut self_verdict: BTreeMap<String, Verdict> = BTreeMap::new();
    let mut evidence: BTreeMap<String, String> = BTreeMap::new();
    for n in &dag.nodes {
        let (mut v, mut ev) = self_verdict_of(&n.id, &n.lint.hard_block, &n.lint.warn, verdicts);
        if stale.contains(&n.id) && v == Verdict::Proven {
            v = Verdict::Unknown;
            ev.push_str("; stale: closure hash changed");
        }
        self_verdict.insert(n.id.clone(), v);
        evidence.insert(n.id.clone(), ev);
    }

    // Demote-only, cycle-safe, memoised propagation.
    let mut colour: BTreeMap<String, Colour> = BTreeMap::new();
    let mut effective: BTreeMap<String, Verdict> = BTreeMap::new();
    let mut cyclic: BTreeSet<String> = BTreeSet::new();
    let ids: Vec<String> = dag.nodes.iter().map(|n| n.id.clone()).collect();
    for id in &ids {
        propagate(
            id,
            &adj,
            &self_verdict,
            &mut colour,
            &mut effective,
            &mut cyclic,
        );
    }

    // Assemble nodes, applying staleness (a stale `Proven` → `Unknown`) and
    // the cycle marker (a node on an import cycle is `Error`/blocked).
    let mut nodes = Vec::with_capacity(dag.nodes.len());
    for n in &dag.nodes {
        let mut eff = *effective.get(&n.id).unwrap_or(&Verdict::Unknown);
        let mut ev = evidence.get(&n.id).cloned().unwrap_or_default();
        if cyclic.contains(&n.id) {
            eff = demote(eff, Verdict::Error);
            ev.push_str("; import cycle");
        }
        nodes.push(ReasonNode {
            id: n.id.clone(),
            self_verdict: *self_verdict.get(&n.id).unwrap_or(&Verdict::Unknown),
            effective: eff,
            verdict_evidence: ev,
            soundness: soundness_of(eff),
            wired: wired.contains(&n.id),
        });
    }

    ReasonDocument {
        version: "0.1",
        dag,
        nodes,
        edges,
        crt_roots,
    }
}

/// Derive reasoning inputs from a workspace's `proven/` state, WITHOUT
/// re-checking: a node whose file basename is in `proven_basenames` gets a
/// `Proven` self-verdict; if that basename is also in `stale_basenames`
/// (content/closure hash changed since promotion) the node id is added to the
/// returned stale set, so [`build`]'s fold demotes it to `Unknown` and
/// propagates. Returns `(verdicts, stale)` for [`build`]. Matched by basename
/// — the workspace stores files flat by basename, so distinct modules that
/// share a basename would collide (a documented limitation).
pub fn workspace_verdicts(
    dag: &DagDocument,
    proven_basenames: &BTreeSet<String>,
    stale_basenames: &BTreeSet<String>,
) -> (BTreeMap<String, Verdict>, BTreeSet<String>) {
    let mut verdicts = BTreeMap::new();
    let mut stale = BTreeSet::new();
    for node in &dag.nodes {
        let base = node
            .file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if proven_basenames.contains(base) {
            verdicts.insert(node.id.clone(), Verdict::Proven);
            if stale_basenames.contains(base) {
                stale.insert(node.id.clone());
            }
        }
    }
    (verdicts, stale)
}

/// The set of ids reachable from any root by following adjacency edges.
fn reachable(roots: &[String], adj: &BTreeMap<String, Vec<(String, Junct)>>) -> BTreeSet<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<String> = roots.to_vec();
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(deps) = adj.get(&id) {
            for (dep, _) in deps {
                if !seen.contains(dep) {
                    stack.push(dep.clone());
                }
            }
        }
    }
    seen
}

/// Post-order demote-fold with grey/black colouring for cycle safety.
/// Returns the effective verdict for `id`, memoising in `effective`. Any
/// node that closes a cycle is recorded in `cyclic` and treated as `Error`.
fn propagate(
    id: &str,
    adj: &BTreeMap<String, Vec<(String, Junct)>>,
    self_verdict: &BTreeMap<String, Verdict>,
    colour: &mut BTreeMap<String, Colour>,
    effective: &mut BTreeMap<String, Verdict>,
    cyclic: &mut BTreeSet<String>,
) -> Verdict {
    match colour.get(id).copied().unwrap_or(Colour::White) {
        Colour::Black => return *effective.get(id).unwrap_or(&Verdict::Unknown),
        // A back-edge to a node still on the stack: a cycle. The importing
        // node is blocked; report `Error` for this leg without recursing.
        Colour::Grey => {
            cyclic.insert(id.to_string());
            return Verdict::Error;
        }
        Colour::White => {}
    }
    colour.insert(id.to_string(), Colour::Grey);

    let base = *self_verdict.get(id).unwrap_or(&Verdict::Unknown);
    let mut eff = base;

    // Group dependency contributions by junct.
    let mut or_best: Option<Verdict> = None;
    if let Some(deps) = adj.get(id) {
        for (dep, junct) in deps {
            let dep_eff = propagate(dep, adj, self_verdict, colour, effective, cyclic);
            match junct {
                // And: every required dep drags us down to the worst of them.
                Junct::And => eff = demote(eff, dep_eff),
                // Or: remember the best alternative; applied after the loop.
                Junct::Or => {
                    or_best = Some(match or_best {
                        Some(b) => promote(b, dep_eff),
                        None => dep_eff,
                    });
                }
            }
        }
    }
    // Or group: the goal needs at least one route, so it is capped at the
    // best available alternative (still demote-only — never above `base`).
    if let Some(best) = or_best {
        eff = demote(eff, best);
    }

    colour.insert(id.to_string(), Colour::Black);
    effective.insert(id.to_string(), eff);
    eff
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build an adjacency map from (from, to, junct) triples for fold tests.
    fn adjacency(edges: &[(&str, &str, Junct)]) -> BTreeMap<String, Vec<(String, Junct)>> {
        let mut adj: BTreeMap<String, Vec<(String, Junct)>> = BTreeMap::new();
        for (f, t, j) in edges {
            adj.entry(f.to_string())
                .or_default()
                .push((t.to_string(), *j));
        }
        adj
    }

    fn fold(
        selfv: &[(&str, Verdict)],
        edges: &[(&str, &str, Junct)],
    ) -> (BTreeMap<String, Verdict>, BTreeSet<String>) {
        let adj = adjacency(edges);
        let sv: BTreeMap<String, Verdict> =
            selfv.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let mut colour = BTreeMap::new();
        let mut effective = BTreeMap::new();
        let mut cyclic = BTreeSet::new();
        for (id, _) in selfv {
            propagate(id, &adj, &sv, &mut colour, &mut effective, &mut cyclic);
        }
        (effective, cyclic)
    }

    #[test]
    fn demote_keeps_the_worse() {
        assert_eq!(
            demote(Verdict::Proven, Verdict::Postulated),
            Verdict::Postulated
        );
        assert_eq!(demote(Verdict::Unknown, Verdict::Proven), Verdict::Unknown);
        assert_eq!(demote(Verdict::Error, Verdict::Refuted), Verdict::Error);
    }

    #[test]
    fn clean_but_unchecked_is_unknown_not_proven() {
        // The core honesty rule: lint-clean + no prover run ⇒ Unknown.
        let verdicts = BTreeMap::new();
        let (v, ev) = self_verdict_of("M", &[], &[], &verdicts);
        assert_eq!(v, Verdict::Unknown);
        assert!(ev.contains("no typecheck run"));
    }

    #[test]
    fn supplied_proven_is_capped_by_postulate_lint() {
        let mut verdicts = BTreeMap::new();
        verdicts.insert("M".to_string(), Verdict::Proven);
        let (v, ev) = self_verdict_of("M", &["unjustified-postulate".to_string()], &[], &verdicts);
        assert_eq!(v, Verdict::Postulated);
        assert!(ev.contains("postulated"));
    }

    #[test]
    fn orphan_lint_does_not_cap_soundness() {
        // orphan-module is a wiring fact, not a soundness one.
        let mut verdicts = BTreeMap::new();
        verdicts.insert("M".to_string(), Verdict::Proven);
        let (v, _) = self_verdict_of("M", &["orphan-module".to_string()], &[], &verdicts);
        assert_eq!(v, Verdict::Proven);
    }

    #[test]
    fn and_dependency_demotes_downstream() {
        // Top imports Mid imports Leaf; Leaf is Postulated (amber). The
        // amber infects the whole And-chain: everyone caps at Postulated.
        let (eff, _) = fold(
            &[
                ("Top", Verdict::Proven),
                ("Mid", Verdict::Proven),
                ("Leaf", Verdict::Postulated),
            ],
            &[("Top", "Mid", Junct::And), ("Mid", "Leaf", Junct::And)],
        );
        assert_eq!(eff["Leaf"], Verdict::Postulated);
        assert_eq!(eff["Mid"], Verdict::Postulated);
        assert_eq!(eff["Top"], Verdict::Postulated);
    }

    #[test]
    fn or_alternative_keeps_a_goal_up() {
        // Goal has two Or routes: one Error, one Proven. The best route
        // wins, so the goal stays Proven (Or is forgiving).
        let (eff, _) = fold(
            &[
                ("Goal", Verdict::Proven),
                ("RouteA", Verdict::Error),
                ("RouteB", Verdict::Proven),
            ],
            &[("Goal", "RouteA", Junct::Or), ("Goal", "RouteB", Junct::Or)],
        );
        assert_eq!(eff["Goal"], Verdict::Proven);
    }

    #[test]
    fn or_group_caps_at_best_when_no_route_is_green() {
        // Both Or routes are amber ⇒ the goal is capped at the best amber.
        let (eff, _) = fold(
            &[
                ("Goal", Verdict::Proven),
                ("RouteA", Verdict::Postulated),
                ("RouteB", Verdict::Admitted),
            ],
            &[("Goal", "RouteA", Junct::Or), ("Goal", "RouteB", Junct::Or)],
        );
        // Admitted (rank 5) beats Postulated (rank 4); demote(Proven, Admitted).
        assert_eq!(eff["Goal"], Verdict::Admitted);
    }

    #[test]
    fn cycle_degrades_to_error_and_terminates() {
        // A <-> B mutual import (illegal in Agda, but possible transiently).
        // The fold must terminate and mark the cycle, not loop forever.
        let (eff, cyclic) = fold(
            &[("A", Verdict::Proven), ("B", Verdict::Proven)],
            &[("A", "B", Junct::And), ("B", "A", Junct::And)],
        );
        assert!(!cyclic.is_empty(), "the cycle must be recorded");
        assert_eq!(eff["A"], Verdict::Error);
        assert_eq!(eff["B"], Verdict::Error);
    }

    #[test]
    fn reachable_marks_only_the_root_cone() {
        let adj = adjacency(&[
            ("All", "Used", Junct::And),
            ("Used", "Deep", Junct::And),
            // Orphan is imported by nobody reachable from All.
        ]);
        let wired = reachable(&["All".to_string()], &adj);
        assert!(wired.contains("All"));
        assert!(wired.contains("Used"));
        assert!(wired.contains("Deep"));
        assert!(!wired.contains("Orphan"));
    }

    // ── reason/0.1 JSON contract freeze (M11) ────────────────────────────
    // The visual layer (arghda-studio) + Groove consumers depend on this
    // shape. A rename here is a BREAKING change — bump the version, don't
    // silently edit. This test pins the schema so that can't happen by
    // accident.
    #[test]
    fn reason_json_contract_is_frozen_at_0_1() {
        use crate::dag::{DagDocument, DagNode, LintSummary};
        use crate::graph::Edge;
        use crate::prover::Agda;
        use std::path::PathBuf;

        let dag = DagDocument {
            version: "0.1",
            include_root: PathBuf::from("/r"),
            entry_modules: vec![PathBuf::from("/r/All.agda")],
            generated_at: "t".to_string(),
            nodes: vec![
                DagNode {
                    id: "All".to_string(),
                    file: PathBuf::from("All.agda"),
                    status: "clean",
                    lint: LintSummary::default(),
                    headlines: vec![],
                },
                DagNode {
                    id: "Util".to_string(),
                    file: PathBuf::from("Util.agda"),
                    status: "clean",
                    lint: LintSummary::default(),
                    headlines: vec![],
                },
            ],
            edges: vec![Edge {
                from: "All".to_string(),
                to: "Util".to_string(),
                kind: "imports",
            }],
            blocked: vec![],
        };
        let doc = build(dag, &Agda, &BTreeMap::new(), &BTreeSet::new());
        let v = serde_json::to_value(&doc).unwrap();

        assert_eq!(v["version"], "0.1");
        assert!(v["dag"].is_object(), "the DAG is embedded verbatim");
        assert!(v["crt_roots"].is_array());
        for k in [
            "id",
            "self_verdict",
            "effective",
            "verdict_evidence",
            "soundness",
            "wired",
        ] {
            assert!(v["nodes"][0].get(k).is_some(), "reason node missing `{k}`");
        }
        for k in ["from", "to", "junct", "kind"] {
            assert!(v["edges"][0].get(k).is_some(), "reason edge missing `{k}`");
        }
    }

    // ── M3 follow-on: workspace-fed verdicts + staleness ─────────────────
    fn two_node_dag() -> crate::dag::DagDocument {
        use crate::dag::{DagDocument, DagNode, LintSummary};
        use crate::graph::Edge;
        use std::path::PathBuf;
        let mk = |id: &str, file: &str| DagNode {
            id: id.to_string(),
            file: PathBuf::from(file),
            status: "clean",
            lint: LintSummary::default(),
            headlines: vec![],
        };
        DagDocument {
            version: "0.1",
            include_root: PathBuf::from("/r"),
            entry_modules: vec![PathBuf::from("/r/All.agda")],
            generated_at: "t".to_string(),
            nodes: vec![mk("All", "All.agda"), mk("Good", "Good.agda")],
            edges: vec![Edge {
                from: "All".to_string(),
                to: "Good".to_string(),
                kind: "imports",
            }],
            blocked: vec![],
        }
    }

    #[test]
    fn workspace_proven_lights_the_cone_and_stale_demotes() {
        use crate::prover::Agda;
        let dag = two_node_dag();
        let proven: BTreeSet<String> = ["Good.agda".to_string()].into_iter().collect();

        // Fresh proven: Good → Proven; All (not proven) → not set.
        let (v, s) = workspace_verdicts(&dag, &proven, &BTreeSet::new());
        assert_eq!(v.get("Good"), Some(&Verdict::Proven));
        assert!(!v.contains_key("All"));
        assert!(s.is_empty());
        let doc = build(dag.clone(), &Agda, &v, &s);
        let eff =
            |d: &ReasonDocument, id: &str| d.nodes.iter().find(|n| n.id == id).unwrap().effective;
        assert_eq!(eff(&doc, "Good"), Verdict::Proven);
        assert_eq!(eff(&doc, "All"), Verdict::Unknown); // not proven itself

        // Stale proven: Good is proven-but-stale → demoted to Unknown, and it
        // drags its importer All down too.
        let stale: BTreeSet<String> = ["Good.agda".to_string()].into_iter().collect();
        let (v2, s2) = workspace_verdicts(&dag, &proven, &stale);
        assert!(s2.contains("Good"));
        let doc2 = build(dag, &Agda, &v2, &s2);
        assert_eq!(
            eff(&doc2, "Good"),
            Verdict::Unknown,
            "stale proven → unknown"
        );
        assert_eq!(eff(&doc2, "All"), Verdict::Unknown);
    }
}
