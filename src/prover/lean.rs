// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! The Lean4 backend.
//!
//! Assistant model, but with a soundness subtlety Lean forces us to be
//! honest about: `lean <file>` elaborates and exits 0 *even when the file
//! contains `sorry`* (it is only a warning). A green exit therefore does NOT
//! mean "proven". Ground-truthed against Lean 4.13.0:
//! * exit non-zero → [`Verdict::Error`] (elaboration failed).
//! * exit 0, output mentions `sorry` → [`Verdict::Admitted`] (a hole rode
//!   along on a green elaboration).
//! * exit 0, clean → [`Verdict::Unknown`] — the file elaborates, but without
//!   a `#print axioms` audit arghda will NOT claim it proven (it could still
//!   use `native_decide`, `sorryAx`, or other axioms). Promoting `Unknown` →
//!   `Proven` via a per-declaration `#print axioms` audit is the follow-on.
//! * binary absent → [`Verdict::Unavailable`].
//!
//! Lean modules are dotted (`Mathlib.Data.Nat` ↔ `Mathlib/Data/Nat.lean`),
//! so [`crate::graph::module_name_of`] is reused. Imports are top-level
//! `import Mod` lines; Lean `open` is a *namespace* directive, not a
//! dependency edge, so it is deliberately ignored. Project-wide module
//! resolution (via `lake env` / `LEAN_PATH`) is a documented follow-on;
//! this baseline runs `lean <file>` directly (core + import-free files).

use super::{Backend, BackendKind, Outcome, Verdict};
use crate::graph;
use crate::lint::{lean::LeanEscapeHatch, LintRule, RuleConfig};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

const TAIL_LINES: usize = 40;

/// The Lean4 theorem prover.
#[derive(Clone, Copy, Debug, Default)]
pub struct Lean;

impl Backend for Lean {
    fn name(&self) -> &'static str {
        "lean4"
    }

    fn kind(&self) -> BackendKind {
        BackendKind::Assistant
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["lean"]
    }

    fn safe_mode(&self) -> Option<&'static str> {
        // Soundness is established per-declaration via `#print axioms`, not a
        // global flag.
        Some("#print axioms")
    }

    fn check_file(&self, file: &Path, _include_root: &Path) -> Result<Outcome> {
        let output = Command::new("lean").arg(file).output();
        match output {
            Ok(out) => {
                let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
                let verdict = lean_verdict(&combined, out.status.success());
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
        }
    }

    fn module_name_of(&self, file: &Path, include_root: &Path) -> Option<String> {
        graph::module_name_of(file, include_root)
    }

    fn module_to_path(&self, module: &str, include_root: &Path) -> PathBuf {
        let mut p = include_root.to_path_buf();
        for part in module.split('.') {
            p.push(part);
        }
        p.set_extension("lean");
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
            let code = trimmed.split(" --").next().unwrap_or(trimmed);
            let tokens: Vec<&str> = code.split_whitespace().collect();
            // Top-level `import Mod`. (`open` is a namespace directive, not
            // an edge — deliberately not matched.)
            if tokens.first() != Some(&"import") {
                continue;
            }
            if let Some(module) = tokens.get(1) {
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
        // Lean's roots are declared in a lakefile (a documented follow-on);
        // the executable convention is `Main.lean`.
        let mut roots = Vec::new();
        for entry in WalkDir::new(include_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("lean") {
                continue;
            }
            if path.file_name().and_then(|s| s.to_str()) == Some("Main.lean") {
                roots.push(path.to_path_buf());
            }
        }
        roots.sort();
        roots
    }

    fn lint_rules(&self, _cfg: &RuleConfig) -> Result<Vec<Box<dyn LintRule>>> {
        Ok(vec![Box::new(LeanEscapeHatch)])
    }

    fn command(&self) -> &'static str {
        "lean"
    }
}

/// Map Lean output + exit status to a [`Verdict`], honestly: a green exit is
/// only `Unknown` (elaborated, un-audited) unless `sorry` is present, which
/// makes it `Admitted`. Never `Proven` without a `#print axioms` audit.
fn lean_verdict(output: &str, exit_ok: bool) -> Verdict {
    if !exit_ok {
        return Verdict::Error;
    }
    // Lean prints `declaration uses 'sorry'` (and `#print axioms` shows
    // `sorryAx`) — parse the tool's own diagnostic, not the source.
    if output.contains("sorry") {
        return Verdict::Admitted;
    }
    Verdict::Unknown
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
    fn lean_backend_identity() {
        assert_eq!(Lean.name(), "lean4");
        assert_eq!(Lean.kind(), BackendKind::Assistant);
        assert_eq!(Lean.extensions(), &["lean"]);
        assert_eq!(Lean.command(), "lean");
    }

    #[test]
    fn module_to_path_uses_lean_extension() {
        assert_eq!(
            Lean.module_to_path("Mathlib.Data.Nat", Path::new("/r")),
            PathBuf::from("/r/Mathlib/Data/Nat.lean")
        );
    }

    #[test]
    fn imports_parsed_open_ignored() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "import Mathlib.Data.Nat\n\
             open Nat\n\
             import Std.Data.List\n\
             -- import Ignored\n",
        )
        .unwrap();
        let imports = Lean.direct_imports(tmp.path()).unwrap();
        assert!(imports.contains(&"Mathlib.Data.Nat".to_string()));
        assert!(imports.contains(&"Std.Data.List".to_string()));
        assert!(!imports.iter().any(|i| i == "Nat"), "`open` is not an edge");
        assert!(!imports.iter().any(|i| i.contains("Ignored")));
    }

    #[test]
    fn verdict_is_honest_about_sorry_and_the_missing_audit() {
        // A green elaboration is only Unknown without a #print axioms audit;
        // a sorry warning makes it Admitted; a failure is Error.
        assert_eq!(lean_verdict("", true), Verdict::Unknown);
        assert_eq!(
            lean_verdict("Foo.lean:1:8: warning: declaration uses 'sorry'", true),
            Verdict::Admitted
        );
        assert_eq!(lean_verdict("error: type mismatch", false), Verdict::Error);
    }

    #[test]
    fn check_file_is_honest_about_availability() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("T.lean");
        std::fs::write(&f, "theorem t : 1 = 1 := rfl\n").unwrap();
        let out = Lean.check_file(&f, tmp.path()).unwrap();
        if out.available {
            // Never fabricated Proven: a clean Lean file is Unknown (audit
            // absent), a sorry file Admitted, a broken file Error.
            assert!(matches!(
                out.verdict,
                Verdict::Unknown | Verdict::Admitted | Verdict::Error
            ));
            assert!(!out.ok, "lean never reports `ok` without an axioms audit");
        } else {
            assert_eq!(out.verdict, Verdict::Unavailable);
        }
    }
}
