// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! The Isabelle/HOL backend.
//!
//! Assistant model. Isabelle is session-based — there is no single-file
//! `--check`; a theory is checked by building a session that lists it. So
//! [`Backend::check_file`] generates a throwaway session: it copies the target
//! `.thy` (and its sibling `.thy` files, so in-tree imports resolve) into a
//! temp directory, writes a `ROOT` declaring `session ArghdaCheck = HOL +
//! theories <Theory>`, and runs `isabelle build -o quick_and_dirty -d <dir>
//! ArghdaCheck`. Heap images land in `~/.isabelle`, so the source tree stays
//! clean. Ground-truthed against Isabelle2025 (HOL heap ships prebuilt, so a
//! per-theory build is ~6 s):
//! * exit non-zero → [`Verdict::Error`] (the theory failed to build — a real
//!   error, or an import the throwaway session could not resolve).
//! * exit 0, source contains `sorry`/`oops` → [`Verdict::Admitted`] (`sorry`
//!   compiles under `quick_and_dirty`, so it must be caught at the source).
//! * exit 0, source contains `axiomatization` → [`Verdict::Postulated`].
//! * exit 0, otherwise → [`Verdict::Proven`].
//! * binary absent → [`Verdict::Unavailable`].
//!
//! Theory names map to files like every other assistant (`Foo` ↔ `Foo.thy`),
//! so [`crate::graph::module_name_of`] is reused (a flat tree's dotted name is
//! just the theory stem). Import edges come from the header `imports` clause
//! (`theory Foo imports Bar Baz begin`). Roots are the theories a `ROOT` file
//! lists under a `theories` section — the genuine Isabelle entry-point
//! convention (analogue of Agda's `All.agda`). Session-qualified / relative-
//! path imports and dependency-ordered multi-session builds are documented
//! follow-ons.

use super::{probe_tool_arg, Backend, BackendKind, Outcome, Probe, Verdict};
use crate::graph;
use crate::lint::{isabelle::IsabelleEscapeHatch, LintRule, RuleConfig};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const TAIL_LINES: usize = 40;

/// `ROOT` section keywords that terminate a `theories` block.
const ROOT_SECTIONS: &[&str] = &[
    "session",
    "options",
    "sessions",
    "directories",
    "theories",
    "document_theories",
    "document_files",
    "export_files",
    "export_classpath",
];

/// The Isabelle/HOL proof assistant.
#[derive(Clone, Copy, Debug, Default)]
pub struct Isabelle;

impl Backend for Isabelle {
    fn name(&self) -> &'static str {
        "isabelle"
    }

    fn kind(&self) -> BackendKind {
        BackendKind::Assistant
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["thy"]
    }

    fn safe_mode(&self) -> Option<&'static str> {
        // No global safe flag; soundness is the absence of sorry/oops and of
        // axiomatization.
        None
    }

    fn check_file(&self, file: &Path, include_root: &Path) -> Result<Outcome> {
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Check")
            .to_string();

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let session_dir =
            std::env::temp_dir().join(format!("arghda-isa-{}-{}", std::process::id(), nanos));
        if fs::create_dir_all(&session_dir).is_err() {
            return Ok(Outcome::unavailable(BackendKind::Assistant));
        }

        // Copy every sibling `.thy` (basename) so in-tree imports resolve,
        // then ensure the target is present (it may sit outside include_root).
        for entry in WalkDir::new(include_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("thy") {
                if let Some(name) = p.file_name() {
                    let _ = fs::copy(p, session_dir.join(name));
                }
            }
        }
        let _ = fs::copy(file, session_dir.join(format!("{stem}.thy")));

        let root = format!("session \"ArghdaCheck\" = \"HOL\" +\n  theories\n    {stem}\n");
        if fs::write(session_dir.join("ROOT"), root).is_err() {
            let _ = fs::remove_dir_all(&session_dir);
            return Ok(Outcome::unavailable(BackendKind::Assistant));
        }

        let output = Command::new("isabelle")
            .arg("build")
            .arg("-o")
            .arg("quick_and_dirty") // let `sorry` compile so we classify it
            .arg("-d")
            .arg(&session_dir)
            .arg("ArghdaCheck")
            .output();

        let _ = fs::remove_dir_all(&session_dir);

        match output {
            Ok(out) => {
                let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
                let src = fs::read_to_string(file).unwrap_or_default();
                let verdict = isabelle_verdict(&src, out.status.success());
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
        p.set_extension("thy");
        p
    }

    fn direct_imports(&self, file: &Path) -> Result<Vec<String>> {
        let contents =
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        Ok(parse_imports(&contents))
    }

    fn discover_roots(&self, include_root: &Path) -> Vec<PathBuf> {
        // The genuine Isabelle convention: a `ROOT` file's `theories` section
        // lists the session's entry theories. Resolve each to its `.thy`.
        let mut roots = Vec::new();
        for entry in WalkDir::new(include_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.file_name().and_then(|s| s.to_str()) == Some("ROOT") {
                roots.extend(root_theory_files(path));
            }
        }
        roots.sort();
        roots.dedup();
        roots
    }

    fn lint_rules(&self, _cfg: &RuleConfig) -> Result<Vec<Box<dyn LintRule>>> {
        Ok(vec![Box::new(IsabelleEscapeHatch)])
    }

    fn command(&self) -> &'static str {
        "isabelle"
    }

    fn probe(&self) -> Probe {
        // Isabelle's version query is the `version` subcommand, not
        // `--version` (which errors with "Unknown Isabelle tool").
        probe_tool_arg(self.name(), self.kind(), self.command(), "version")
    }
}

/// Map Isabelle source + build status to a [`Verdict`], honestly: a green
/// build is `Proven` unless the source carries an escape hatch —
/// `sorry`/`oops` ⇒ `Admitted` (they compile under `quick_and_dirty`, so the
/// exit code alone would hide them), `axiomatization` ⇒ `Postulated`. A
/// non-zero build ⇒ `Error`. Admitted dominates a postulate.
fn isabelle_verdict(src: &str, exit_ok: bool) -> Verdict {
    if !exit_ok {
        return Verdict::Error;
    }
    if has_token(src, "sorry") || has_token(src, "oops") {
        return Verdict::Admitted;
    }
    if has_token(src, "axiomatization") {
        return Verdict::Postulated;
    }
    Verdict::Proven
}

/// Extract the theories named in a theory header's `imports` clause:
/// `theory Foo imports Bar Baz "…/Qux" begin`. Bare names pass through;
/// quoted path imports contribute their final path segment. Everything from
/// `imports` up to `begin` (or a `keywords`/`abbrevs` header section) is
/// scanned, tolerating a multi-line header.
fn parse_imports(contents: &str) -> Vec<String> {
    let tokens: Vec<&str> = contents.split_whitespace().collect();
    let Some(start) = tokens.iter().position(|t| *t == "imports") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for tok in &tokens[start + 1..] {
        if *tok == "begin" || *tok == "keywords" || *tok == "abbrevs" {
            break;
        }
        let name = theory_token(tok);
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

/// Normalise an `imports`/`ROOT` theory token to a bare theory name: strip
/// quotes, then take the final path segment (`"../Other/Baz"` → `Baz`).
fn theory_token(tok: &str) -> String {
    let t = tok.trim_matches('"');
    let seg = t.rsplit('/').next().unwrap_or(t);
    seg.trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
        .to_string()
}

/// Parse a `ROOT` file's `theories` sections and resolve each listed theory
/// to a `<ROOT dir>/<name>.thy` file that exists. Tolerant: option groups
/// `(…)` and other sections are skipped.
fn root_theory_files(root_file: &Path) -> Vec<PathBuf> {
    let Ok(contents) = fs::read_to_string(root_file) else {
        return Vec::new();
    };
    let dir = root_file.parent().unwrap_or(Path::new("."));
    let tokens: Vec<&str> = contents.split_whitespace().collect();
    let mut out = Vec::new();
    let mut collecting = false;
    let mut skip_group = false;
    for tok in tokens {
        // Skip an inline `(options …)` group inside a theories section.
        if skip_group {
            if tok.ends_with(')') {
                skip_group = false;
            }
            continue;
        }
        if tok == "theories" {
            collecting = true;
            continue;
        }
        if ROOT_SECTIONS.contains(&tok) {
            collecting = false;
            continue;
        }
        if !collecting {
            continue;
        }
        if tok.starts_with('(') {
            if !tok.ends_with(')') {
                skip_group = true;
            }
            continue;
        }
        if tok == "+" || tok == "=" {
            continue;
        }
        let name = theory_token(tok);
        if name.is_empty() {
            continue;
        }
        let candidate = dir.join(format!("{name}.thy"));
        if candidate.is_file() {
            out.push(candidate);
        }
    }
    out
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// `tok` appears in `s` as a delimited token.
fn has_token(s: &str, tok: &str) -> bool {
    s.split(|c: char| c.is_whitespace() || "(){}[];,.\"".contains(c))
        .any(|w| w == tok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isabelle_backend_identity() {
        assert_eq!(Isabelle.name(), "isabelle");
        assert_eq!(Isabelle.kind(), BackendKind::Assistant);
        assert_eq!(Isabelle.extensions(), &["thy"]);
        assert_eq!(Isabelle.command(), "isabelle");
    }

    #[test]
    fn module_to_path_uses_thy_extension() {
        assert_eq!(
            Isabelle.module_to_path("Foo", Path::new("/r")),
            PathBuf::from("/r/Foo.thy")
        );
    }

    #[test]
    fn imports_clause_parsed_multiline_and_quoted() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "theory Foo\n  imports Main Bar \"../Other/Baz\"\n  keywords \"x\"\nbegin\nend\n",
        )
        .unwrap();
        let imports = Isabelle.direct_imports(tmp.path()).unwrap();
        assert!(imports.contains(&"Main".to_string()));
        assert!(imports.contains(&"Bar".to_string()));
        assert!(
            imports.contains(&"Baz".to_string()),
            "quoted path → final seg"
        );
        // `keywords` terminates the imports clause.
        assert!(!imports.iter().any(|i| i == "x"));
    }

    #[test]
    fn verdict_is_honest_about_sorry_axiom_and_clean() {
        assert_eq!(
            isabelle_verdict("lemma l: \"True\" by simp", false),
            Verdict::Error
        );
        assert_eq!(
            isabelle_verdict("lemma l: \"P\" sorry", true),
            Verdict::Admitted
        );
        assert_eq!(
            isabelle_verdict("lemma l: \"P\" oops", true),
            Verdict::Admitted
        );
        assert_eq!(
            isabelle_verdict("axiomatization where ax: \"P\"", true),
            Verdict::Postulated
        );
        assert_eq!(
            isabelle_verdict("lemma l: \"True\" by simp", true),
            Verdict::Proven
        );
    }

    #[test]
    fn root_theories_are_discovered_as_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        std::fs::write(r.join("All.thy"), "theory All imports Main begin end\n").unwrap();
        std::fs::write(r.join("Extra.thy"), "theory Extra imports Main begin end\n").unwrap();
        std::fs::write(
            r.join("ROOT"),
            "session \"Demo\" = \"HOL\" +\n  options [document = false]\n  theories\n    All\n    Extra\n  document_files\n    \"root.tex\"\n",
        )
        .unwrap();
        let roots = Isabelle.discover_roots(r);
        let names: Vec<String> = roots
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"All.thy".to_string()));
        assert!(names.contains(&"Extra.thy".to_string()));
        assert_eq!(roots.len(), 2, "only theories-section entries: {names:?}");
    }

    #[test]
    fn check_file_is_honest_about_availability() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("T.thy");
        std::fs::write(
            &f,
            "theory T imports Main begin\nlemma l: \"True\" by simp\nend\n",
        )
        .unwrap();
        let out = Isabelle.check_file(&f, tmp.path()).unwrap();
        if out.available {
            assert!(matches!(
                out.verdict,
                Verdict::Proven | Verdict::Admitted | Verdict::Postulated | Verdict::Error
            ));
            assert_eq!(out.ok, out.verdict == Verdict::Proven);
        } else {
            assert_eq!(out.verdict, Verdict::Unavailable);
            assert!(!out.ok);
        }
    }
}
