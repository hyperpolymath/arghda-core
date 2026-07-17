// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! The Mizar backend.
//!
//! Assistant model, and — unlike Coq/Lean/Isabelle — Mizar has **no** `sorry`/
//! `admit`-style escape hatch: an incomplete or wrong proof is simply a
//! verifier error, and a normal article cannot introduce axioms (only the MML
//! system articles carry the built-in axioms). So the verdict is essentially
//! binary and the lint pack is empty by design; soundness is the verifier's.
//!
//! Mizar checks in two steps against the MML data dir (`MIZFILES`): `accom`
//! accommodates the `environ` block, then `verifier` checks the article and
//! writes errors to a sibling `<name>.err` file — an EMPTY `.err` is the
//! authoritative "verified clean" signal (Mizar reports errors in that file,
//! though the verifier also exits non-zero on error). Ground-truthed against
//! Mizar 8.1.15 (statically-linked i386, runs on x86_64):
//! * `MIZFILES` unset → [`Verdict::Unavailable`] (the MML data dir is the
//!   verifier's precondition; without it no honest check is possible).
//! * `verifier`/`accom` binary absent → [`Verdict::Unavailable`].
//! * ran, `<name>.err` empty and exit 0 → [`Verdict::Proven`].
//! * ran, `<name>.err` non-empty (or exit non-zero) → [`Verdict::Error`].
//!
//! [`Backend::check_file`] copies the target `.miz` (and sibling `.miz` files,
//! best-effort for local cross-references) into a temp dir so the source tree
//! stays clean. Article names are the file stem (`ordinal1` ↔ `ordinal1.miz`),
//! so [`crate::graph::module_name_of`] is reused. Import edges come from the
//! `environ` block's directives (`vocabularies`/`notations`/`theorems`/…),
//! lower-cased to match the on-disk stem; only in-tree articles become edges
//! (the MML is external). Local-article cross-references are supported:
//! [`Backend::check_file`] exports each in-tree dependency in topological order
//! (accom → verifier → exporter → transfer, populating a local `./prel` the
//! accommodator reads) before checking the target, so a `theorems X;` on a
//! sibling article `X` resolves. A session/root convention remains a follow-on.

use super::{probe_tool_arg, Backend, BackendKind, Outcome, Probe, Verdict};
use crate::graph;
use crate::lint::{LintRule, RuleConfig};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const TAIL_LINES: usize = 40;

/// The `environ` directives that reference other articles.
const ENVIRON_DIRECTIVES: &[&str] = &[
    "vocabularies",
    "notations",
    "constructors",
    "registrations",
    "definitions",
    "expansions",
    "equalities",
    "theorems",
    "schemes",
    "requirements",
];

/// The Mizar proof checker.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mizar;

impl Backend for Mizar {
    fn name(&self) -> &'static str {
        "mizar"
    }

    fn kind(&self) -> BackendKind {
        BackendKind::Assistant
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["miz"]
    }

    fn safe_mode(&self) -> Option<&'static str> {
        // Mizar has no admit/sorry construct; soundness is the verifier.
        None
    }

    fn check_file(&self, file: &Path, include_root: &Path) -> Result<Outcome> {
        // The MML data dir is the verifier's precondition. Without it no honest
        // check is possible, so report Unavailable (with the reason) rather
        // than run a doomed verify and mislabel it Error.
        let Ok(mizfiles) = std::env::var("MIZFILES") else {
            return Ok(Outcome {
                available: false,
                exit_code: None,
                ok: false,
                output_tail: "MIZFILES not set — Mizar needs its MML data dir".to_string(),
                kind: BackendKind::Assistant,
                verdict: Verdict::Unavailable,
            });
        };

        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("article")
            .to_string();

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let work =
            std::env::temp_dir().join(format!("arghda-miz-{}-{}", std::process::id(), nanos));
        if fs::create_dir_all(&work).is_err() {
            return Ok(Outcome::unavailable(BackendKind::Assistant));
        }

        // Copy sibling articles (best-effort for local refs), then the target.
        for entry in WalkDir::new(include_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("miz") {
                if let Some(name) = p.file_name() {
                    let _ = fs::copy(p, work.join(name));
                }
            }
        }
        let _ = fs::copy(file, work.join(format!("{stem}.miz")));

        // Build the target's in-tree dependencies into a local library so its
        // `theorems`/`definitions`/… directives resolve: for each dep in
        // topological order (deps before dependents), run the export pipeline
        // accom → verifier → exporter → transfer, which populates `./prel` in
        // the work dir (the accommodator reads local library files from there).
        // External MML articles are skipped (resolved from MIZFILES). Verdict-
        // affecting failures surface honestly as a non-empty target `.err`.
        let src = fs::read_to_string(file).unwrap_or_default();
        let deps = mizar_transitive_deps(&parse_environ_imports(&src), include_root);
        for dep in &deps {
            for tool in ["accom", "verifier", "exporter", "transfer"] {
                let _ = Command::new(tool)
                    .arg(dep)
                    .current_dir(&work)
                    .env("MIZFILES", &mizfiles)
                    .output();
            }
        }

        // Step 1: accommodate the environment.
        let accom = Command::new("accom")
            .arg(&stem)
            .current_dir(&work)
            .env("MIZFILES", &mizfiles)
            .output();
        if let Err(e) = &accom {
            let _ = fs::remove_dir_all(&work);
            if e.kind() == std::io::ErrorKind::NotFound {
                return Ok(Outcome::unavailable(BackendKind::Assistant));
            }
        }

        // Step 2: verify.
        let output = Command::new("verifier")
            .arg(&stem)
            .current_dir(&work)
            .env("MIZFILES", &mizfiles)
            .output();

        let result = match output {
            Ok(out) => {
                let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
                let err_path = work.join(format!("{stem}.err"));
                let err_len = fs::metadata(&err_path).map(|m| m.len()).unwrap_or(1);
                // Authoritative: an empty `.err` (with a clean exit) is verified.
                let verdict = if out.status.success() && err_len == 0 {
                    Verdict::Proven
                } else {
                    Verdict::Error
                };
                // On error, surface the first error line(s) from `.err`.
                if verdict == Verdict::Error {
                    if let Ok(errs) = fs::read_to_string(&err_path) {
                        if !errs.trim().is_empty() {
                            combined.push_str("\n.err: ");
                            combined.push_str(errs.lines().next().unwrap_or("").trim());
                        }
                    }
                }
                Ok(Outcome {
                    available: true,
                    exit_code: out.status.code(),
                    ok: verdict == Verdict::Proven,
                    output_tail: tail(&combined, TAIL_LINES),
                    kind: BackendKind::Assistant,
                    verdict,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Outcome::unavailable(BackendKind::Assistant))
            }
            Err(e) => Err(e.into()),
        };

        let _ = fs::remove_dir_all(&work);
        result
    }

    fn module_name_of(&self, file: &Path, include_root: &Path) -> Option<String> {
        graph::module_name_of(file, include_root)
    }

    fn module_to_path(&self, module: &str, include_root: &Path) -> PathBuf {
        let mut p = include_root.to_path_buf();
        for part in module.split('.') {
            p.push(part);
        }
        p.set_extension("miz");
        p
    }

    fn direct_imports(&self, file: &Path) -> Result<Vec<String>> {
        let contents =
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        Ok(parse_environ_imports(&contents))
    }

    fn discover_roots(&self, _include_root: &Path) -> Vec<PathBuf> {
        // Mizar has no aggregator / entry-article convention (articles are
        // listed in a project's text/ dir); an empty root set is honest.
        Vec::new()
    }

    fn lint_rules(&self, _cfg: &RuleConfig) -> Result<Vec<Box<dyn LintRule>>> {
        // Mizar has no sorry/admit escape hatch; nothing to warn on.
        Ok(Vec::new())
    }

    fn command(&self) -> &'static str {
        "verifier"
    }

    fn probe(&self) -> Probe {
        // Mizar's version query is `verifier -v` (prints the banner).
        probe_tool_arg(self.name(), self.kind(), self.command(), "-v")
    }
}

/// The in-tree transitive article dependencies of a Mizar file, given the
/// articles its `environ` block references, in dependency-first (topological)
/// order — so exporting the list left-to-right satisfies every later `accom`.
/// External MML articles (absent under `include_root`) are skipped. Cycle-safe
/// via the visited set.
fn mizar_transitive_deps(direct: &[String], include_root: &Path) -> Vec<String> {
    let mut visited = std::collections::HashSet::new();
    let mut order = Vec::new();
    for m in direct {
        mizar_visit_dep(m, include_root, &mut visited, &mut order);
    }
    order
}

fn mizar_visit_dep(
    module: &str,
    include_root: &Path,
    visited: &mut std::collections::HashSet<String>,
    order: &mut Vec<String>,
) {
    if !visited.insert(module.to_string()) {
        return;
    }
    // Only in-tree articles resolve to a readable `.miz`; MML articles are
    // left for MIZFILES.
    let Ok(src) = fs::read_to_string(include_root.join(format!("{module}.miz"))) else {
        return;
    };
    for imp in parse_environ_imports(&src) {
        mizar_visit_dep(&imp, include_root, visited, order);
    }
    order.push(module.to_string());
}

/// Extract the articles referenced by a Mizar `environ` block's directives
/// (`vocabularies`/`notations`/`theorems`/…), lower-cased to match on-disk
/// stems. `::` line comments are stripped; only the region between `environ`
/// and the body's `begin` is scanned.
fn parse_environ_imports(contents: &str) -> Vec<String> {
    // Strip `::` line comments, then work on the whitespace token stream.
    let decommented: String = contents
        .lines()
        .map(|l| l.split("::").next().unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n");
    let tokens: Vec<&str> = decommented.split_whitespace().collect();

    let Some(env_pos) = tokens.iter().position(|t| *t == "environ") else {
        return Vec::new();
    };
    let end = tokens[env_pos + 1..]
        .iter()
        .position(|t| *t == "begin")
        .map(|p| env_pos + 1 + p)
        .unwrap_or(tokens.len());

    let mut out = Vec::new();
    for tok in &tokens[env_pos + 1..end] {
        if ENVIRON_DIRECTIVES.contains(tok) {
            continue;
        }
        let name: String = tok
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_')
            .collect::<String>()
            .to_ascii_lowercase();
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mizar_backend_identity() {
        assert_eq!(Mizar.name(), "mizar");
        assert_eq!(Mizar.kind(), BackendKind::Assistant);
        assert_eq!(Mizar.extensions(), &["miz"]);
        assert_eq!(Mizar.command(), "verifier");
    }

    #[test]
    fn module_to_path_uses_miz_extension() {
        assert_eq!(
            Mizar.module_to_path("ordinal1", Path::new("/r")),
            PathBuf::from("/r/ordinal1.miz")
        );
    }

    #[test]
    fn environ_directives_parsed_and_lowercased() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "environ\n\
             :: a comment mentioning THEOREMS\n\
             vocabularies XBOOLE_0, SUBSET_1;\n\
             notations XBOOLE_0;\n\
             theorems TARSKI, XBOOLE_0; :: trailing comment\n\
             begin\n\
             theorem for X being set holds X = X;\n",
        )
        .unwrap();
        let imports = Mizar.direct_imports(tmp.path()).unwrap();
        assert!(imports.contains(&"xboole_0".to_string()));
        assert!(imports.contains(&"subset_1".to_string()));
        assert!(imports.contains(&"tarski".to_string()));
        // Directive keywords are not imports; deduped.
        assert!(!imports
            .iter()
            .any(|i| i == "vocabularies" || i == "theorems"));
        assert_eq!(
            imports.iter().filter(|i| *i == "xboole_0").count(),
            1,
            "deduped"
        );
    }

    #[test]
    fn definitions_directive_parsed_and_lowercased() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "environ\n\
             definitions FUNCT_1, RELAT_1;\n\
             begin\n\
             definition\n\
               let X;\n\
               func id X -> Function of X,X := id X;\n\
             end;\n",
        )
        .unwrap();
        let imports = Mizar.direct_imports(tmp.path()).unwrap();
        assert!(imports.contains(&"funct_1".to_string()));
        assert!(imports.contains(&"relat_1".to_string()));
        // `definitions` keyword is not an import.
        assert!(!imports.iter().any(|i| i == "definitions"));
    }

    #[test]
    fn all_environ_directives_parsed() {
        // Test that all ENVIRON_DIRECTIVES are correctly skipped (not imported).
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "environ\n\
             vocabularies VOCAB;\n\
             notations NOT;\n\
             constructors CONS;\n\
             registrations REG;\n\
             definitions DEF;\n\
             expansions EXP;\n\
             equalities EQ;\n\
             theorems THM;\n\
             schemes SCH;\n\
             requirements REQ;\n\
             begin\n",
        )
        .unwrap();
        let imports = Mizar.direct_imports(tmp.path()).unwrap();
        // Verify that all directives are present as imports (lowercased).
        assert!(imports.contains(&"vocab".to_string()));
        assert!(imports.contains(&"not".to_string()));
        assert!(imports.contains(&"cons".to_string()));
        assert!(imports.contains(&"reg".to_string()));
        assert!(imports.contains(&"def".to_string()));
        assert!(imports.contains(&"exp".to_string()));
        assert!(imports.contains(&"eq".to_string()));
        assert!(imports.contains(&"thm".to_string()));
        assert!(imports.contains(&"sch".to_string()));
        assert!(imports.contains(&"req".to_string()));
        // Verify that keyword directives themselves are not imported.
        assert!(!imports.iter().any(|i| {
            i == "vocabularies"
                || i == "notations"
                || i == "constructors"
                || i == "registrations"
                || i == "definitions"
                || i == "expansions"
                || i == "equalities"
                || i == "theorems"
                || i == "schemes"
                || i == "requirements"
        }));
    }

    #[test]
    fn no_environ_yields_no_imports() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "begin\ntheorem X;\n").unwrap();
        assert!(Mizar.direct_imports(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn mizar_has_no_lint_rules() {
        // Mizar has no admit/sorry construct — the lint pack is empty by design.
        assert!(Mizar.lint_rules(&RuleConfig::default()).unwrap().is_empty());
    }

    #[test]
    fn transitive_deps_are_topologically_ordered() {
        // z requires y; y requires x; x is a leaf. Build order for z must be
        // [x, y] (deps before dependents); MML refs are skipped.
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        std::fs::write(r.join("x.miz"), "environ\nbegin\n").unwrap();
        std::fs::write(r.join("y.miz"), "environ\n theorems X;\nbegin\n").unwrap();
        std::fs::write(r.join("z.miz"), "environ\n theorems Y, XBOOLE_0;\nbegin\n").unwrap();
        let deps =
            mizar_transitive_deps(&parse_environ_imports("environ\n theorems Z;\nbegin\n"), r);
        assert_eq!(
            deps,
            vec!["x".to_string(), "y".to_string(), "z".to_string()]
        );
        // The MML article is not staged as an in-tree dep.
        assert!(!deps.iter().any(|m| m == "xboole_0"));
    }

    #[test]
    fn check_file_is_honest_about_availability() {
        // Never a fabricated pass, whatever the environment: with MIZFILES
        // unset or the binary absent → Unavailable; with both present → a real
        // Proven/Error from the verifier's `.err`. `ok` iff Proven.
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("art.miz");
        std::fs::write(&f, "environ\nbegin\n").unwrap();
        let out = Mizar.check_file(&f, tmp.path()).unwrap();
        assert!(matches!(
            out.verdict,
            Verdict::Proven | Verdict::Error | Verdict::Unavailable
        ));
        assert_eq!(out.ok, out.verdict == Verdict::Proven);
    }
}
