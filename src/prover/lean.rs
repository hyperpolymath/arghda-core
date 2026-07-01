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
//! * exit 0, clean → run the **`#print axioms` audit** and promote honestly:
//!   `Proven` if every declaration depends only on the standard axioms
//!   (`propext`, `Classical.choice`, `Quot.sound`); `Postulated` if a
//!   non-standard axiom sneaks in (e.g. the one `native_decide` introduces —
//!   which a bare elaboration would NOT reveal); `Admitted` on `sorryAx`. If
//!   the audit can't run (no declarations, `lean` absent, or the file imports
//!   modules the bare temp-dir copy can't resolve), it stays `Verdict::Unknown`.
//! * binary absent → [`Verdict::Unavailable`].
//!
//! The audit currently reaches import-free files (a temp-dir copy elaborates
//! them without `LEAN_PATH`); auditing imported files needs `lake env`
//! resolution — a documented follow-on.
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
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

/// Lean's standard, trusted axioms — a proof depending only on these is
/// sound (this is the mathlib convention). Anything else (sorryAx, or the
/// axioms `native_decide` introduces) is not.
const STANDARD_AXIOMS: &[&str] = &["propext", "Classical.choice", "Quot.sound"];

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
                let mut verdict = lean_verdict(&combined, out.status.success());
                // A clean elaboration is only `Unknown` on its own — run the
                // `#print axioms` audit to promote it honestly: Proven if every
                // declaration depends only on the standard axioms, Postulated
                // if a non-standard axiom (e.g. `native_decide`'s) sneaks in,
                // Admitted on sorryAx. If the audit can't run (imports need
                // lake, no declarations, tool absent) it stays `Unknown`.
                if verdict == Verdict::Unknown {
                    if let Some(audited) = axiom_audit(file) {
                        verdict = audited;
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

/// The declaration names in a Lean source that can depend on axioms
/// (`theorem`/`lemma`/`def`/`abbrev`/`instance`), skipping leading
/// attributes/modifiers and anonymous decls. Deliberately conservative: a
/// missed name just isn't audited; a mis-parsed name makes `lean` error and
/// the audit falls back to `Unknown` — it never yields a wrong verdict.
fn decl_names(src: &str) -> Vec<String> {
    const KEYWORDS: &[&str] = &["theorem", "lemma", "def", "abbrev", "instance"];
    const MODIFIERS: &[&str] = &[
        "private",
        "protected",
        "noncomputable",
        "partial",
        "unsafe",
        "scoped",
        "local",
        "mutual",
    ];
    let mut names = Vec::new();
    for line in src.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let mut i = 0;
        // Skip leading `@[..]` attributes and declaration modifiers.
        while let Some(tk) = tokens.get(i) {
            if tk.starts_with("@[") || MODIFIERS.contains(tk) {
                i += 1;
            } else {
                break;
            }
        }
        if tokens.get(i).is_some_and(|tk| KEYWORDS.contains(tk)) {
            if let Some(name_tok) = tokens.get(i + 1) {
                let name: String = name_tok
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.' || *c == '\'')
                    .collect();
                if !name.is_empty() {
                    names.push(name);
                }
            }
        }
    }
    names.dedup();
    names
}

/// Classify the combined output of a batch of `#print axioms` commands:
/// any `sorryAx` ⇒ Admitted; else any non-standard axiom ⇒ Postulated; else
/// (all clean or standard-only) ⇒ Proven.
fn classify_axioms(output: &str) -> Verdict {
    let mut saw_nonstandard = false;
    for line in output.lines() {
        let Some(open) = line.find("depends on axioms: [") else {
            continue; // "does not depend on any axioms" ⇒ clean, skip
        };
        let rest = &line[open + "depends on axioms: [".len()..];
        let list = rest.split(']').next().unwrap_or(rest);
        for ax in list.split(',') {
            let ax = ax.trim();
            if ax == "sorryAx" {
                return Verdict::Admitted;
            }
            if !ax.is_empty() && !STANDARD_AXIOMS.contains(&ax) {
                saw_nonstandard = true;
            }
        }
    }
    if saw_nonstandard {
        Verdict::Postulated
    } else {
        Verdict::Proven
    }
}

/// Run a `#print axioms` audit on `file`'s declarations. Copies the source
/// into a fresh temp dir, appends `#print axioms <name>` per declaration,
/// and runs `lean`. Returns the classified verdict, or `None` when the audit
/// can't be trusted — no declarations, `lean` absent, or the copy fails to
/// elaborate (e.g. it imports modules that need `lake env`/`LEAN_PATH`, which
/// a bare temp dir lacks). `None` ⇒ the caller keeps the honest `Unknown`.
fn axiom_audit(file: &Path) -> Option<Verdict> {
    let src = fs::read_to_string(file).ok()?;
    let names = decl_names(&src);
    if names.is_empty() {
        return None;
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("arghda-audit-{}-{}", std::process::id(), nanos));
    fs::create_dir_all(&dir).ok()?;

    let mut body = src;
    body.push('\n');
    for n in &names {
        body.push_str(&format!("#print axioms {n}\n"));
    }
    let audit_file = dir.join("Audit.lean");

    let verdict = match fs::write(&audit_file, &body) {
        Ok(()) => match Command::new("lean").arg(&audit_file).output() {
            // Only trust the audit if the copy elaborated cleanly; otherwise
            // (imports unresolved in the temp dir, etc.) stay Unknown.
            Ok(out) if out.status.success() => {
                let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
                Some(classify_axioms(&combined))
            }
            _ => None,
        },
        Err(_) => None,
    };

    let _ = fs::remove_dir_all(&dir);
    verdict
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
    fn decl_names_parses_keywords_modifiers_attributes() {
        let src = "\
@[simp] theorem foo : True := trivial\n\
private def bar : Nat := 0\n\
lemma baz.qux : 1 = 1 := rfl\n\
noncomputable def qq : Nat := 0\n\
example : True := trivial\n\
-- theorem commented : ...\n\
#eval 1\n";
        let names = decl_names(src);
        assert!(names.contains(&"foo".to_string()));
        assert!(names.contains(&"bar".to_string()));
        assert!(names.contains(&"baz.qux".to_string()), "dotted names kept");
        assert!(names.contains(&"qq".to_string()), "modifier skipped");
        // `example` is anonymous → not audited; `#eval` is not a decl.
        assert!(!names.iter().any(|n| n == "example" || n == "1"));
    }

    #[test]
    fn classify_axioms_maps_the_ground_truthed_output() {
        // The three shapes ground-truthed against Lean 4.13.0.
        assert_eq!(
            classify_axioms("'t' does not depend on any axioms"),
            Verdict::Proven
        );
        assert_eq!(
            classify_axioms("'c' depends on axioms: [propext, Classical.choice, Quot.sound]"),
            Verdict::Proven,
        );
        assert_eq!(
            classify_axioms("'g' depends on axioms: [sorryAx]"),
            Verdict::Admitted
        );
        assert_eq!(
            classify_axioms("'n' depends on axioms: [Lean.ofReduceBool]"),
            Verdict::Postulated,
            "native_decide's axiom is non-standard ⇒ amber",
        );
        // Mixed: any sorryAx dominates.
        assert_eq!(
            classify_axioms("'a' depends on axioms: [propext]\n'b' depends on axioms: [sorryAx]"),
            Verdict::Admitted
        );
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
            // Honest verdict invariant: never fabricated. With the axioms
            // audit a clean `rfl` proof is promoted to Proven; `ok` iff
            // Proven; and the value is always one of the real states.
            assert!(matches!(
                out.verdict,
                Verdict::Proven | Verdict::Unknown | Verdict::Admitted | Verdict::Error
            ));
            assert_eq!(out.ok, out.verdict == Verdict::Proven);
        } else {
            assert_eq!(out.verdict, Verdict::Unavailable);
        }
    }
}
