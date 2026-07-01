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
        // Idris2 has no `All.agda`-style convention; the executable entry is
        // `Main.idr`. (`.ipkg`-declared `main`/`modules` roots are a
        // documented follow-on.)
        let mut roots = Vec::new();
        for entry in WalkDir::new(include_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("idr") {
                continue;
            }
            if path.file_name().and_then(|s| s.to_str()) == Some("Main.idr") {
                roots.push(path.to_path_buf());
            }
        }
        roots.sort();
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
}
