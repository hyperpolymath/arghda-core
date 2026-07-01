// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! First-class Agda import graph.
//!
//! The reachability primitives (`module_name_of`, `module_to_path`,
//! `direct_imports`, `transitive_imports`) used to live private inside the
//! `orphan-module` lint rule, which computed the edges and then threw them
//! away. They are promoted here so the `dag` command (and any downstream
//! visual layer) can consume the actual dependency graph. The orphan rule
//! now reuses these.
//!
//! Edges are kept only to modules that resolve to a file *inside* the
//! include root: the graph is the project's internal dependency DAG, so
//! stdlib / external imports are intentionally omitted.

use crate::prover::Backend;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Relative module path for `file` under `include_root`, dotted
/// (`Ordinal/Closure.agda` → `Ordinal.Closure`). `None` if `file` is
/// outside the root or has a non-normal component.
pub fn module_name_of(file: &Path, include_root: &Path) -> Option<String> {
    let rel = file.strip_prefix(include_root).ok()?;
    let stem = rel.with_extension("");
    let mut parts = Vec::new();
    for comp in stem.components() {
        let std::path::Component::Normal(s) = comp else {
            return None;
        };
        parts.push(s.to_str()?.to_string());
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("."))
}

/// Inverse of [`module_name_of`]: dotted module name → file path.
pub fn module_to_path(module: &str, include_root: &Path) -> PathBuf {
    let mut p = include_root.to_path_buf();
    for part in module.split('.') {
        p.push(part);
    }
    p.set_extension("agda");
    p
}

/// Extract module names appearing in `import …` / `open import …`
/// top-level forms of `file`. Tolerant: takes the first token after
/// `import`; ignores `hiding`/`using`/`as`/`public` modifiers and `--`
/// line comments.
pub fn direct_imports(file: &Path) -> Result<Vec<String>> {
    let contents =
        fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let mut out = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") {
            continue;
        }
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        let Some(i) = tokens.iter().position(|&t| t == "import") else {
            continue;
        };
        // Require `import` to be top-level or directly after `open`, so we
        // don't trip on the word appearing mid-expression.
        if i > 0 && tokens[i - 1] != "open" {
            continue;
        }
        let Some(module) = tokens.get(i + 1) else {
            continue;
        };
        let cleaned =
            module.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '_');
        if !cleaned.is_empty() {
            out.push(cleaned.to_string());
        }
    }
    Ok(out)
}

/// Set of module names transitively reachable from `entry` by following
/// `open import` edges. Missing files (stdlib / external) are skipped.
pub fn transitive_imports(entry: &Path, include_root: &Path) -> Result<HashSet<String>> {
    let mut reachable: HashSet<String> = HashSet::new();
    let mut worklist: Vec<String> = Vec::new();

    if let Some(m) = module_name_of(entry, include_root) {
        reachable.insert(m.clone());
        worklist.push(m);
    } else {
        for imp in direct_imports(entry)? {
            if reachable.insert(imp.clone()) {
                worklist.push(imp);
            }
        }
    }

    while let Some(module) = worklist.pop() {
        let path = module_to_path(&module, include_root);
        if !path.is_file() {
            continue;
        }
        for imp in direct_imports(&path)? {
            if reachable.insert(imp.clone()) {
                worklist.push(imp);
            }
        }
    }

    Ok(reachable)
}

/// Union of the modules reachable from each root in `roots`. A module
/// verified from *any* CI entry point counts as reachable.
pub fn reachable_from_roots(roots: &[PathBuf], include_root: &Path) -> Result<HashSet<String>> {
    let mut all = HashSet::new();
    for root in roots {
        for m in transitive_imports(root, include_root)? {
            all.insert(m);
        }
    }
    Ok(all)
}

/// Discover conventional CI root modules under `include_root`: every file
/// named `All.agda` or `Smoke.agda`, at any depth. These are the entry
/// points an estate `--safe --without-K` workspace registers its verified
/// suite from (e.g. echo-types has `All.agda`, `Smoke.agda`,
/// `characteristic/All.agda`, `examples/All.agda`, `tutorial/All.agda`).
pub fn discover_roots(include_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for entry in WalkDir::new(include_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("agda") {
            continue;
        }
        if matches!(
            path.file_name().and_then(|s| s.to_str()),
            Some("All.agda") | Some("Smoke.agda")
        ) {
            roots.push(path.to_path_buf());
        }
    }
    roots.sort();
    roots
}

/// A node in the import graph: a `.agda` source file and its module name.
#[derive(Clone, Debug, Serialize)]
pub struct GraphNode {
    pub id: String,
    /// Path relative to the include root.
    pub file: PathBuf,
}

/// An `imports` edge from one in-tree module to another.
#[derive(Clone, Debug, Serialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: &'static str,
}

/// The internal import DAG of an Agda source tree.
#[derive(Clone, Debug, Serialize)]
pub struct ImportGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
}

/// Walk every source file the `backend` claims (by extension) under
/// `include_root` and build the internal import graph, using the backend's
/// per-language module-name / import parsing. Output is deterministic
/// (nodes and edges are sorted), which keeps the emitted DAG stable for
/// diffing and tests. A solver backend with no import notion yields an
/// edge-free set of isolated nodes, which is valid.
pub fn build(include_root: &Path, backend: &dyn Backend) -> Result<ImportGraph> {
    let exts = backend.extensions();
    // module id -> relative file path, for every in-tree module.
    let mut by_id: BTreeMap<String, PathBuf> = BTreeMap::new();
    for entry in WalkDir::new(include_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let is_source = path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|e| exts.contains(&e));
        if !is_source {
            continue;
        }
        if let Some(id) = backend.module_name_of(path, include_root) {
            let rel = path
                .strip_prefix(include_root)
                .unwrap_or(path)
                .to_path_buf();
            by_id.insert(id, rel);
        }
    }

    let nodes: Vec<GraphNode> = by_id
        .iter()
        .map(|(id, file)| GraphNode {
            id: id.clone(),
            file: file.clone(),
        })
        .collect();

    let mut edges = Vec::new();
    for id in by_id.keys() {
        let path = backend.module_to_path(id, include_root);
        for imp in backend.direct_imports(&path)? {
            // Keep only edges to modules that exist in-tree.
            if by_id.contains_key(&imp) {
                edges.push(Edge {
                    from: id.clone(),
                    to: imp,
                    kind: "imports",
                });
            }
        }
    }
    edges.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
    edges.dedup_by(|a, b| a.from == b.from && a.to == b.to);

    Ok(ImportGraph { nodes, edges })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_roundtrip() {
        let root = Path::new("/r");
        let file = Path::new("/r/Ordinal/Closure.agda");
        let name = module_name_of(file, root).unwrap();
        assert_eq!(name, "Ordinal.Closure");
        assert_eq!(
            module_to_path(&name, root),
            PathBuf::from("/r/Ordinal/Closure.agda")
        );
    }

    #[test]
    fn parses_open_import_with_modifiers() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "module M where\n\
             open import Data.Nat using (ℕ)\n\
             open import Foo.Bar\n\
             import Baz as B\n\
             -- open import IgnoredComment\n",
        )
        .unwrap();
        let imports = direct_imports(tmp.path()).unwrap();
        assert!(imports.contains(&"Data.Nat".to_string()));
        assert!(imports.contains(&"Foo.Bar".to_string()));
        assert!(imports.contains(&"Baz".to_string()));
        assert!(!imports.iter().any(|i| i.contains("Ignored")));
    }

    #[test]
    fn transitive_imports_terminates_on_a_cycle() {
        // A <-> B mutual import is illegal in Agda, but a half-written file
        // under active edit in `working/` can transiently produce one. The
        // `reachable` visited-set must stop the walk from looping forever.
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        std::fs::write(r.join("A.agda"), "module A where\nopen import B\n").unwrap();
        std::fs::write(r.join("B.agda"), "module B where\nopen import A\n").unwrap();
        let reach = transitive_imports(&r.join("A.agda"), r).unwrap();
        assert!(reach.contains("A"));
        assert!(reach.contains("B"));
    }

    #[test]
    fn discover_and_reach_over_multiple_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        std::fs::write(r.join("All.agda"), "module All where\nopen import Used\n").unwrap();
        std::fs::write(
            r.join("Smoke.agda"),
            "module Smoke where\nopen import Other\n",
        )
        .unwrap();
        std::fs::write(r.join("Used.agda"), "module Used where\n").unwrap();
        std::fs::write(r.join("Other.agda"), "module Other where\n").unwrap();
        std::fs::write(r.join("Orphan.agda"), "module Orphan where\n").unwrap();

        let roots = discover_roots(r);
        let names: Vec<String> = roots
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(roots.len(), 2, "discovers All.agda + Smoke.agda: {names:?}");
        assert!(names.contains(&"All.agda".to_string()));
        assert!(names.contains(&"Smoke.agda".to_string()));

        // Reachability is the UNION: Used (via All) and Other (via Smoke)
        // are both reachable; Orphan (via neither) is not.
        let reachable = reachable_from_roots(&roots, r).unwrap();
        assert!(reachable.contains("Used"));
        assert!(reachable.contains("Other"));
        assert!(!reachable.contains("Orphan"));
    }
}
