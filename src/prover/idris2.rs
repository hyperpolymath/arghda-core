// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! The Idris2 backend.
//!
//! Idris2 is a core estate language — it owns ABIs (the Idris2-ABI / Zig-FFI
//! pattern), so it is a first-class backend, not merely a prover. This
//! adapter shells out to `idris2 --check --source-dir <root> <file>` (the
//! `-i`-style include-root analog, ground-truthed against Idris2 0.7.0).
//!
//! Assistant model, exit-code-only: exit 0 → [`Verdict::Proven`], a run that
//! errors → [`Verdict::Error`], an absent binary → [`Verdict::Unavailable`].
//! arghda never claims a result Idris2 did not return.
//!
//! Idris2 modules are dotted and map to paths exactly like Agda
//! (`Data.Vect` ↔ `Data/Vect.idr`), so [`crate::graph::module_name_of`] is
//! reused; only the file extension (`.idr`) and the import syntax
//! (`import [public] Mod [as Alias]`, top-level — no `open`) differ.

use super::{Backend, BackendKind, Outcome, Verdict};
use crate::graph;
use crate::lint::{idris2::Idris2EscapeHatch, LintRule, RuleConfig};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

const TAIL_LINES: usize = 40;

/// The Idris2 proof assistant / dependently-typed language.
#[derive(Clone, Copy, Debug, Default)]
pub struct Idris2;

impl Backend for Idris2 {
    fn name(&self) -> &'static str {
        "idris2"
    }

    fn kind(&self) -> BackendKind {
        BackendKind::Assistant
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["idr"]
    }

    fn safe_mode(&self) -> Option<&'static str> {
        // Idris2 has no global `--safe`; totality is per-definition, opted in
        // file-wide with this directive.
        Some("%default total")
    }

    fn check_file(&self, file: &Path, include_root: &Path) -> Result<Outcome> {
        let output = Command::new("idris2")
            .arg("--check")
            .arg("--source-dir")
            .arg(include_root)
            .arg(file)
            .output();

        match output {
            Ok(out) => {
                let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
                let ok = out.status.success();
                Ok(Outcome {
                    available: true,
                    exit_code: out.status.code(),
                    ok,
                    output_tail: tail(&combined, TAIL_LINES),
                    kind: BackendKind::Assistant,
                    verdict: if ok { Verdict::Proven } else { Verdict::Error },
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Outcome::unavailable(BackendKind::Assistant))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn module_name_of(&self, file: &Path, include_root: &Path) -> Option<String> {
        // Extension-agnostic; identical dotted convention to Agda.
        graph::module_name_of(file, include_root)
    }

    fn module_to_path(&self, module: &str, include_root: &Path) -> PathBuf {
        let mut p = include_root.to_path_buf();
        for part in module.split('.') {
            p.push(part);
        }
        p.set_extension("idr");
        p
    }

    fn direct_imports(&self, file: &Path) -> Result<Vec<String>> {
        let contents =
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let mut out = Vec::new();
        for line in contents.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("--") {
                continue;
            }
            // Strip a trailing line comment, then tokenise.
            let code = trimmed.split(" --").next().unwrap_or(trimmed);
            let tokens: Vec<&str> = code.split_whitespace().collect();
            // Idris2 imports are top-level: `import [public] Mod [as Alias]`.
            if tokens.first() != Some(&"import") {
                continue;
            }
            let mut idx = 1;
            if tokens.get(idx) == Some(&"public") {
                idx += 1;
            }
            if let Some(module) = tokens.get(idx) {
                let cleaned =
                    module.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '_');
                if !cleaned.is_empty() {
                    out.push(cleaned.to_string());
                }
            }
        }
        Ok(out)
    }

    fn discover_roots(&self, include_root: &Path) -> Vec<PathBuf> {
        // Two conventions, unioned:
        //   1. `.ipkg`-declared `main = <Module>` — the package's executable
        //      entry, resolved to its `.idr` file (needn't be `Main.idr`).
        //   2. Any `Main.idr` — the bare-tree fallback.
        let mut roots = Vec::new();
        for entry in WalkDir::new(include_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            match path.extension().and_then(|s| s.to_str()) {
                Some("ipkg") => roots.extend(ipkg_main_roots(path, include_root)),
                Some("idr") if path.file_name().and_then(|s| s.to_str()) == Some("Main.idr") => {
                    roots.push(path.to_path_buf());
                }
                _ => {}
            }
        }
        roots.sort();
        roots.dedup();
        roots
    }

    fn lint_rules(&self, _cfg: &RuleConfig) -> Result<Vec<Box<dyn LintRule>>> {
        Ok(vec![Box::new(Idris2EscapeHatch)])
    }

    fn command(&self) -> &'static str {
        "idris2"
    }
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Resolve an Idris2 `.ipkg`'s `main = <Module>` declaration(s) to the
/// module's `.idr` file, honouring an optional `sourcedir`. Returns the
/// resolved files that exist (first matching candidate base per `main`).
/// Non-`main` fields are ignored; `--` comments are stripped.
fn ipkg_main_roots(ipkg: &Path, include_root: &Path) -> Vec<PathBuf> {
    let Ok(contents) = fs::read_to_string(ipkg) else {
        return Vec::new();
    };
    let mut mains: Vec<String> = Vec::new();
    let mut sourcedir: Option<String> = None;
    for line in contents.lines() {
        let code = line.split("--").next().unwrap_or(line);
        if let Some((key, value)) = code.split_once('=') {
            match key.trim() {
                "main" => mains.push(value.trim().to_string()),
                "sourcedir" | "source-dir" => {
                    sourcedir = Some(value.trim().trim_matches('"').trim().to_string());
                }
                _ => {}
            }
        }
    }

    let ipkg_dir = ipkg.parent().unwrap_or(include_root);
    // Candidate source roots to resolve the main module against, in order.
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Some(sd) = &sourcedir {
        bases.push(ipkg_dir.join(sd));
        bases.push(include_root.join(sd));
    }
    bases.push(ipkg_dir.to_path_buf());
    bases.push(include_root.to_path_buf());

    let mut out = Vec::new();
    for m in &mains {
        for base in &bases {
            let mut p = base.clone();
            for part in m.split('.') {
                p.push(part);
            }
            p.set_extension("idr");
            if p.is_file() {
                out.push(p);
                break; // first existing candidate wins
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idris2_backend_identity() {
        assert_eq!(Idris2.name(), "idris2");
        assert_eq!(Idris2.kind(), BackendKind::Assistant);
        assert_eq!(Idris2.extensions(), &["idr"]);
    }

    #[test]
    fn module_to_path_uses_idr_extension() {
        let root = Path::new("/r");
        assert_eq!(
            Idris2.module_to_path("Data.Vect", root),
            PathBuf::from("/r/Data/Vect.idr")
        );
    }

    #[test]
    fn parses_idris_imports_including_public_and_alias() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "module Main\n\
             import Data.Vect\n\
             import public Data.List\n\
             import Util as U\n\
             -- import Ignored.Comment\n\
             x = 0 -- import AlsoIgnored\n",
        )
        .unwrap();
        let imports = Idris2.direct_imports(tmp.path()).unwrap();
        assert!(imports.contains(&"Data.Vect".to_string()));
        assert!(imports.contains(&"Data.List".to_string()), "public import");
        assert!(imports.contains(&"Util".to_string()), "aliased import");
        assert!(!imports.iter().any(|i| i.contains("Ignored")));
        assert!(!imports.iter().any(|i| i.contains("Also")));
    }

    #[test]
    fn check_file_is_honest_about_availability() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("Main.idr");
        std::fs::write(&f, "module Main\n\nmain : IO ()\nmain = pure ()\n").unwrap();
        let out = Idris2.check_file(&f, tmp.path()).unwrap();
        if out.available {
            assert!(matches!(out.verdict, Verdict::Proven | Verdict::Error));
            assert_eq!(out.ok, out.verdict == Verdict::Proven);
        } else {
            assert_eq!(out.verdict, Verdict::Unavailable);
            assert!(!out.ok);
            assert!(out.exit_code.is_none());
        }
    }

    #[test]
    fn ipkg_main_resolves_via_sourcedir_and_strips_comments() {
        // main declared with a dotted module under a quoted sourcedir, plus a
        // `--` comment and a distractor field — only the real main resolves.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src/Cli")).unwrap();
        std::fs::write(root.join("src/Cli/App.idr"), "module Cli.App\n").unwrap();
        std::fs::write(
            root.join("thing.ipkg"),
            "package thing\n\
             -- main = NotThis   (this line is a comment)\n\
             sourcedir = \"src\"\n\
             main = Cli.App\n\
             executable = thing\n",
        )
        .unwrap();
        let roots = ipkg_main_roots(&root.join("thing.ipkg"), root);
        assert_eq!(roots, vec![root.join("src/Cli/App.idr")]);
    }

    #[test]
    fn ipkg_with_no_resolvable_main_yields_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("x.ipkg"), "package x\nmain = Nope\n").unwrap();
        assert!(ipkg_main_roots(&root.join("x.ipkg"), root).is_empty());
    }
}
