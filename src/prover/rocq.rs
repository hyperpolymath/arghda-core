// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! The Coq / Rocq backend.
//!
//! Assistant model, with the same soundness subtlety Lean forces on us: a
//! green `coqc` exit does NOT by itself mean "proven". `coqc` compiles a file
//! that closes proofs with `Admitted.` (which turns the goal into an axiom) or
//! that declares `Axiom`/`Parameter`/`Conjecture` postulates — all exit 0.
//! So the honest verdict, ground-truthed against Coq 8.18.0, is:
//! * exit non-zero → [`Verdict::Error`] (the kernel rejected something, or an
//!   import could not be resolved — a bare single-file `coqc` cannot build a
//!   dependency chain; that is a documented limitation, not a proof).
//! * exit 0, source contains `Admitted`/`admit` → [`Verdict::Admitted`] (an
//!   unfinished proof rode along on a green compile).
//! * exit 0, source contains a genuine unverified postulate → [`Verdict::Postulated`].
//! * exit 0, otherwise → [`Verdict::Proven`]: `coqc` kernel-checks every `Qed`
//!   at compile time, so a clean compile with no escape hatch *is* a checked
//!   proof (stronger than Lean, where a bare elaboration needs a `#print
//!   axioms` audit).
//! * binary absent → [`Verdict::Unavailable`].
//!
//! "Genuine unverified postulate" is decided by a Section-aware classifier
//! ([`count_genuine_postulates`]) ported from panic-attack's
//! `count_rocq_unverified_postulates`: `Variable`/`Hypothesis`/`Parameter`
//! inside a `Section … End` are discharged at `End` (not counted), and
//! module-level `Parameter` declarations whose stated type is a carrier
//! (`Type`/`Set`), a decidability witness, or a non-`Prop` function symbol are
//! abstractions awaiting instantiation (not counted). Everything else —
//! notably `Prop`-valued `Axiom`/`Parameter` — is a real postulate.
//!
//! Coq logical names are dotted and map to paths like every other assistant
//! (`Foo.Bar` ↔ `Foo/Bar.v`), so [`crate::graph::module_name_of`] is reused.
//! Import edges come only from `Require` (with or without `Import`/`Export`,
//! and the `From P Require …` form); a bare `Import`/`Export` is a namespace
//! directive, not a dependency edge (as Lean's `open` is), so it is ignored.
//! `_CoqProject`-driven logical-path resolution and dependency-ordered
//! compilation are documented follow-ons; this baseline compiles a single
//! file with `-R <root> ""` and honestly reports `Error` when a dependency it
//! cannot build is required.

use super::{Backend, BackendKind, Outcome, Verdict};
use crate::graph;
use crate::lint::{coq::CoqEscapeHatch, LintRule, RuleConfig};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const TAIL_LINES: usize = 40;

/// The Coq / Rocq proof assistant.
#[derive(Clone, Copy, Debug, Default)]
pub struct Coq;

impl Backend for Coq {
    fn name(&self) -> &'static str {
        "coq"
    }

    fn kind(&self) -> BackendKind {
        BackendKind::Assistant
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["v"]
    }

    fn safe_mode(&self) -> Option<&'static str> {
        // Coq has no global safe flag; soundness is the absence of
        // Admitted/admit and of genuine Axiom/Parameter postulates (the
        // runtime analogue is `Print Assumptions`).
        Some("Print Assumptions")
    }

    fn check_file(&self, file: &Path, include_root: &Path) -> Result<Outcome> {
        // Redirect the compiled `.vo` into a temp *directory* so the source
        // tree stays clean, and map the include root to the empty logical
        // prefix so a sibling `Require` at least has a chance to resolve.
        // `coqc -o` insists the target basename equal the source's (only the
        // directory may differ), so keep the stem and vary the directory.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let out_dir =
            std::env::temp_dir().join(format!("arghda-coq-{}-{}", std::process::id(), nanos));
        let _ = fs::create_dir_all(&out_dir);
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("Check");
        let vo_out = out_dir.join(format!("{stem}.vo"));

        let output = Command::new("coqc")
            .arg("-q") // ignore any coqrc
            .arg("-no-glob") // don't drop a .glob next to the source
            .arg("-R")
            .arg(include_root)
            .arg("")
            .arg("-o")
            .arg(&vo_out)
            .arg(file)
            .output();

        let _ = fs::remove_dir_all(&out_dir);

        match output {
            Ok(out) => {
                let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
                let src = fs::read_to_string(file).unwrap_or_default();
                let verdict = coq_verdict(&src, out.status.success());
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
        p.set_extension("v");
        p
    }

    fn direct_imports(&self, file: &Path) -> Result<Vec<String>> {
        let contents =
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        Ok(parse_requires(&contents))
    }

    fn discover_roots(&self, include_root: &Path) -> Vec<PathBuf> {
        // Coq has no universal entry-module convention; the estate aggregator
        // convention is `All.v` (mirroring Agda's `All.agda`). `_CoqProject`-
        // declared file sets are a documented follow-on.
        let mut roots = Vec::new();
        for entry in WalkDir::new(include_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("v") {
                continue;
            }
            if path.file_name().and_then(|s| s.to_str()) == Some("All.v") {
                roots.push(path.to_path_buf());
            }
        }
        roots.sort();
        roots
    }

    fn lint_rules(&self, _cfg: &RuleConfig) -> Result<Vec<Box<dyn LintRule>>> {
        Ok(vec![Box::new(CoqEscapeHatch)])
    }

    fn command(&self) -> &'static str {
        "coqc"
    }
}

/// Map Coq source + exit status to a [`Verdict`], honestly: a green compile is
/// `Proven` (the kernel checked every `Qed`) *unless* an escape hatch is
/// present — `Admitted`/`admit` ⇒ `Admitted`, a genuine postulate ⇒
/// `Postulated`. A non-zero exit ⇒ `Error`. Admitted dominates a postulate
/// (an unfinished proof is the more severe hole), mirroring Lean's `sorryAx`.
fn coq_verdict(src: &str, exit_ok: bool) -> Verdict {
    if !exit_ok {
        return Verdict::Error;
    }
    if has_admit(src) {
        return Verdict::Admitted;
    }
    if count_genuine_postulates(src) > 0 {
        return Verdict::Postulated;
    }
    Verdict::Proven
}

/// Whether `src` closes a proof with `Admitted.` or uses the `admit` tactic —
/// scanned as delimited tokens over comment-stripped lines so `admittedly` or
/// a `(* admit *)` comment does not match.
fn has_admit(src: &str) -> bool {
    for raw in src.lines() {
        let line = strip_line_comments(raw);
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        for tok in ["Admitted", "admit"] {
            if has_token(&line, tok) {
                return true;
            }
        }
    }
    false
}

/// Extract the modules a Coq file depends on via `Require` statements.
///
/// Handles `Require [Import|Export] M1 M2 ….` and `From P Require [Import|
/// Export] M ….` (the `P.M` form is recorded as the bare `M`, since a flat
/// in-tree layout resolves `M.v`; logical-prefix resolution is a follow-on).
/// A bare `Import`/`Export` (namespace directive, not a dependency) is ignored.
/// Statements are assumed to sit on a single logical line (the common case).
fn parse_requires(contents: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in contents.lines() {
        let line = strip_line_comments(raw);
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(req_idx) = tokens.iter().position(|t| *t == "Require") else {
            continue;
        };
        // `From P Require …` is valid; a `Require` anywhere else on the line
        // is still the dependency keyword. Modules follow, after an optional
        // `Import`/`Export`.
        let mut idx = req_idx + 1;
        if matches!(tokens.get(idx), Some(&"Import") | Some(&"Export")) {
            idx += 1;
        }
        for tok in &tokens[idx..] {
            // The statement terminator is a lone `.` or a trailing `.` on the
            // last module token.
            let ends = tok.ends_with('.');
            let cleaned = clean_module(tok);
            if !cleaned.is_empty() {
                out.push(cleaned);
            }
            if ends {
                break;
            }
        }
    }
    out
}

/// Trim a module token to its dotted identifier: drop a single trailing
/// statement-terminator `.` and any surrounding punctuation, keeping internal
/// dots (`Coq.Lists.List.` → `Coq.Lists.List`).
fn clean_module(tok: &str) -> String {
    let t = tok.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '_');
    t.strip_suffix('.')
        .unwrap_or(t)
        .trim_matches('.')
        .to_string()
}

/// Count Coq `Axiom`/`Parameter` declarations that are genuine unverified
/// postulates, excluding the two legitimate scaffold shapes (ported from
/// panic-attack's `count_rocq_unverified_postulates`):
///
/// 1. **Section-scoped declarations** between `Section name` and matching
///    `End name` — discharged at `End`, so not floating assumptions.
/// 2. **Module-level abstraction parameters** whose stated type is a carrier
///    (`Type`/`Set`), a decidability witness, or a non-`Prop` function symbol.
///
/// Everything else is counted, notably `Prop`-valued `Axiom`/`Parameter`.
/// Conservative: unknown shapes default to counted.
fn count_genuine_postulates(code: &str) -> usize {
    let mut section_depth: i32 = 0;
    let mut count = 0usize;
    let mut pending: Option<String> = None;

    for raw_line in code.lines() {
        let stripped = strip_line_comments(raw_line);
        let trimmed = stripped.trim_start();

        if trimmed.starts_with("Section ") {
            section_depth += 1;
            pending = None;
            continue;
        }
        if trimmed.starts_with("End ") && trimmed.contains('.') && section_depth > 0 {
            section_depth -= 1;
            pending = None;
            continue;
        }
        if section_depth > 0 {
            pending = None;
            continue;
        }

        // Continuation of a multi-line declaration.
        if let Some(mut p) = pending.take() {
            p.push(' ');
            p.push_str(stripped.trim());
            if p.contains('.') {
                if !is_abstraction_parameter(&p) {
                    count += 1;
                }
            } else {
                pending = Some(p);
            }
            continue;
        }

        let is_decl = trimmed.starts_with("Axiom ") || trimmed.starts_with("Parameter ");
        if !is_decl {
            continue;
        }
        if stripped.trim_end().ends_with('.') {
            if !is_abstraction_parameter(&stripped) {
                count += 1;
            }
        } else {
            pending = Some(stripped.trim().to_string());
        }
    }
    count
}

/// Classify a Coq `Axiom`/`Parameter` declaration as an abstraction parameter
/// (carrier type, decidability witness, or non-`Prop` function symbol) rather
/// than a postulate. Conservative: unknown shapes return `false` (counted).
fn is_abstraction_parameter(decl: &str) -> bool {
    let Some(colon) = decl.find(':') else {
        return false;
    };
    let typ = decl[colon + 1..].trim();
    let typ = typ.strip_suffix('.').unwrap_or(typ).trim();

    // 1. Carrier type.
    if typ == "Type" || typ == "Set" {
        return true;
    }
    // 2. Decidability witness `forall …, { _ = _ } + { _ <> _ }`.
    if typ.contains('{')
        && typ.contains('=')
        && typ.contains('}')
        && typ.contains('+')
        && typ.contains("<>")
    {
        return true;
    }
    // 3. Function type with a clearly non-`Prop` codomain.
    if typ.contains("->") {
        let return_type = typ.rsplit("->").next().unwrap_or("").trim();
        if return_type == "Prop" {
            return false;
        }
        let first_word = return_type.split_whitespace().next().unwrap_or("");
        const CONCRETE_RETURNS: &[&str] = &[
            "Q", "R", "Z", "N", "nat", "bool", "list", "option", "prod", "sum", "unit", "Type",
            "Set",
        ];
        if CONCRETE_RETURNS.contains(&first_word) {
            return true;
        }
        if let Some(c) = first_word.chars().next() {
            if c.is_ascii_lowercase() {
                return true;
            }
        }
    }
    false
}

/// Remove Coq `(* … *)` comment spans from a single line. Balanced spans are
/// cut out; an unmatched `(*` truncates the rest of the line (a multi-line
/// comment's interior is a documented limitation).
fn strip_line_comments(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut depth = 0u32;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'(' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
            continue;
        }
        if depth > 0 && i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b')' {
            depth -= 1;
            i += 2;
            continue;
        }
        if depth == 0 {
            out.push(bytes[i] as char);
        }
        i += 1;
    }
    out
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// `tok` appears in `s` as a delimited token (so `admittedly` or a
/// namespaced `Foo.admit` do not match the bare keyword).
fn has_token(s: &str, tok: &str) -> bool {
    s.split(|c: char| c.is_whitespace() || "(){};,.".contains(c))
        .any(|w| w == tok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coq_backend_identity() {
        assert_eq!(Coq.name(), "coq");
        assert_eq!(Coq.kind(), BackendKind::Assistant);
        assert_eq!(Coq.extensions(), &["v"]);
        assert_eq!(Coq.command(), "coqc");
    }

    #[test]
    fn module_to_path_uses_v_extension() {
        assert_eq!(
            Coq.module_to_path("Coq.Lists.List", Path::new("/r")),
            PathBuf::from("/r/Coq/Lists/List.v")
        );
    }

    #[test]
    fn requires_parsed_import_export_and_from() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "Require Import Coq.Lists.List.\n\
             Require Export Helper.\n\
             From Stdlib Require Import Arith.\n\
             Require A B.\n\
             Import Namespace.\n\
             Export Other.\n\
             (* Require Import Commented. *)\n",
        )
        .unwrap();
        let imports = Coq.direct_imports(tmp.path()).unwrap();
        assert!(imports.contains(&"Coq.Lists.List".to_string()));
        assert!(imports.contains(&"Helper".to_string()), "Require Export");
        assert!(imports.contains(&"Arith".to_string()), "From … Require");
        assert!(imports.contains(&"A".to_string()) && imports.contains(&"B".to_string()));
        // Bare Import/Export are namespace directives, not edges.
        assert!(!imports.iter().any(|i| i == "Namespace" || i == "Other"));
        assert!(!imports.iter().any(|i| i.contains("Commented")));
    }

    #[test]
    fn verdict_is_honest_about_admit_axiom_and_clean() {
        // Non-zero exit is always Error, whatever the source says.
        assert_eq!(
            coq_verdict("Theorem t : True. Proof. exact I. Qed.\n", false),
            Verdict::Error
        );
        // Admitted / admit ⇒ Admitted (amber), and dominates a postulate.
        assert_eq!(
            coq_verdict("Theorem t : True.\nProof.\nAdmitted.\n", true),
            Verdict::Admitted
        );
        assert_eq!(
            coq_verdict(
                "Axiom bad : False.\nTheorem t : True.\nProof.\nadmit.\nDefined.\n",
                true
            ),
            Verdict::Admitted
        );
        // A genuine Prop-valued Axiom ⇒ Postulated.
        assert_eq!(
            coq_verdict("Axiom em : forall P : Prop, P \\/ ~P.\n", true),
            Verdict::Postulated
        );
        // Clean compile with only kernel-checked proofs ⇒ Proven.
        assert_eq!(
            coq_verdict("Theorem t : True.\nProof.\nexact I.\nQed.\n", true),
            Verdict::Proven
        );
    }

    #[test]
    fn section_scoped_declarations_are_discharged() {
        let code = "\
Section OrderedField.\n\
  Variable R : Type.\n\
  Hypothesis Rplus_comm : forall x y : R, x = y.\n\
  Parameter Rabs : R -> R.\n\
End OrderedField.\n";
        assert_eq!(
            count_genuine_postulates(code),
            0,
            "Section-scoped Variable/Hypothesis/Parameter discharge at End"
        );
    }

    #[test]
    fn module_level_abstractions_are_not_postulates() {
        assert_eq!(count_genuine_postulates("Parameter State : Type.\n"), 0);
        assert_eq!(
            count_genuine_postulates("Parameter dec : forall x y : State, {x = y} + {x <> y}.\n"),
            0
        );
        assert_eq!(
            count_genuine_postulates("Parameter kernel : State -> State -> Q.\n"),
            0
        );
    }

    #[test]
    fn prop_valued_axioms_are_counted() {
        assert_eq!(
            count_genuine_postulates("Axiom excluded_middle : forall P : Prop, P \\/ ~P.\n"),
            1
        );
        // Bare `Parameter foo.` with no type is an unknown shape ⇒ counted.
        assert_eq!(count_genuine_postulates("Parameter foo.\n"), 1);
    }

    #[test]
    fn check_file_is_honest_about_availability() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("T.v");
        std::fs::write(&f, "Theorem t : True.\nProof.\nexact I.\nQed.\n").unwrap();
        let out = Coq.check_file(&f, tmp.path()).unwrap();
        if out.available {
            assert!(matches!(
                out.verdict,
                Verdict::Proven | Verdict::Admitted | Verdict::Postulated | Verdict::Error
            ));
            assert_eq!(out.ok, out.verdict == Verdict::Proven);
        } else {
            assert_eq!(out.verdict, Verdict::Unavailable);
            assert!(!out.ok);
            assert!(out.exit_code.is_none());
        }
    }
}
