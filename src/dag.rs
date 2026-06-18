//! The DAG document — the JSON contract a visual layer consumes.
//!
//! Builds on the import graph (`crate::graph`) plus a lint pass over every
//! node. The schema follows `docs/arghda-spec.adoc`; `status` is honest
//! about what the engine knows without a typecheck: it is lint-derived
//! (`clean` / `warn` / `blocked`), not a claim of `proven`. Triage status
//! (proven/rejected) attaches when files flow through a workspace and are
//! checked with Agda.

use crate::diagnostic::Severity;
use crate::graph::{self, Edge};
use crate::lint::{run_lints, LintContext, LintRule};
use crate::timestamp::now_rfc3339;
use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Per-node lint summary: the rule names that fired, by severity.
#[derive(Clone, Debug, Default, Serialize)]
pub struct LintSummary {
    pub hard_block: Vec<String>,
    pub warn: Vec<String>,
}

/// A DAG node: an in-tree module with its lint-derived status.
#[derive(Clone, Debug, Serialize)]
pub struct DagNode {
    pub id: String,
    pub file: PathBuf,
    /// `clean` | `warn` | `blocked` (lint-derived; not a proof claim).
    pub status: &'static str,
    pub lint: LintSummary,
}

/// A module that cannot advance, and why.
#[derive(Clone, Debug, Serialize)]
pub struct Blocked {
    pub node: String,
    pub blocked_by: Vec<String>,
    pub reason: String,
}

/// The full emitted document.
#[derive(Clone, Debug, Serialize)]
pub struct DagDocument {
    pub version: &'static str,
    pub include_root: PathBuf,
    pub entry_modules: Vec<PathBuf>,
    pub generated_at: String,
    pub nodes: Vec<DagNode>,
    pub edges: Vec<Edge>,
    pub blocked: Vec<Blocked>,
}

/// Build the DAG document for the source tree at `include_root`, using
/// `entry_modules` (the union of CI roots) for the orphan-reachability rule
/// and `rules` as the lint pack.
pub fn build(
    include_root: &Path,
    entry_modules: &[PathBuf],
    rules: &[Box<dyn LintRule>],
) -> Result<DagDocument> {
    let graph = graph::build(include_root)?;
    let ctx = LintContext {
        include_root,
        entry_modules,
    };

    let mut nodes = Vec::with_capacity(graph.nodes.len());
    let mut self_blocked: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for gn in &graph.nodes {
        let abs = include_root.join(&gn.file);
        let report = run_lints(&abs, &ctx, rules)?;

        let mut summary = LintSummary::default();
        for d in &report.diagnostics {
            match d.severity {
                Severity::HardBlock => summary.hard_block.push(d.rule.clone()),
                Severity::Warn => summary.warn.push(d.rule.clone()),
            }
        }
        summary.hard_block.sort();
        summary.hard_block.dedup();
        summary.warn.sort();
        summary.warn.dedup();

        let status = if !summary.hard_block.is_empty() {
            self_blocked.insert(gn.id.clone(), summary.hard_block.clone());
            "blocked"
        } else if !summary.warn.is_empty() {
            "warn"
        } else {
            "clean"
        };

        nodes.push(DagNode {
            id: gn.id.clone(),
            file: gn.file.clone(),
            status,
            lint: summary,
        });
    }

    let blocked = compute_blocked(&self_blocked, &graph.edges);

    Ok(DagDocument {
        version: "0.1",
        include_root: include_root.to_path_buf(),
        entry_modules: entry_modules.to_vec(),
        generated_at: now_rfc3339(),
        nodes,
        edges: graph.edges,
        blocked,
    })
}

/// A node is blocked if it hard-blocks on its own (`reason` = the rule
/// names) or if any module it imports is itself hard-blocked
/// (`blocked_by` = those prerequisites). Both are surfaced so the visual
/// layer can colour a node red for its own fault *and* show the upstream
/// wall it is waiting on.
fn compute_blocked(self_blocked: &BTreeMap<String, Vec<String>>, edges: &[Edge]) -> Vec<Blocked> {
    let mut out = Vec::new();

    for (node, rules) in self_blocked {
        out.push(Blocked {
            node: node.clone(),
            blocked_by: Vec::new(),
            reason: rules.join(", "),
        });
    }

    // Group each node's hard-blocked direct prerequisites.
    let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for e in edges {
        if self_blocked.contains_key(&e.to) {
            deps.entry(e.from.clone()).or_default().insert(e.to.clone());
        }
    }
    for (node, prereqs) in deps {
        out.push(Blocked {
            node,
            blocked_by: prereqs.into_iter().collect(),
            reason: "prerequisite not clean".to_string(),
        });
    }

    out.sort_by(|a, b| (&a.node, &a.reason).cmp(&(&b.node, &b.reason)));
    out
}
