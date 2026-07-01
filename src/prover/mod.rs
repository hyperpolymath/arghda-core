// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Prover/solver backend abstraction — the seam that makes arghda
//! prover-parametric.
//!
//! ArghDA never proves anything itself; a [`Backend`] is a thin, honest
//! adapter over an external tool. Two interaction models are unified here:
//! *assistants* (typecheck a file → exit code is the verdict) and *solvers*
//! (feed a query → parse `sat`/`unsat`/`unknown`). Both map their real,
//! observed result into a common [`Verdict`].
//!
//! Hard rule (owner directive + AGENTIC.a2ml): a backend NEVER reports a
//! verdict the tool did not emit. [`Backend::check_file`] derives its
//! verdict from the actual exit code / parsed output, and degrades to
//! [`Verdict::Unavailable`] when the tool is absent rather than pretending
//! success. This is the module boundary where that honesty is enforced.
//!
//! Everything backend-neutral — the four-state machine (`workspace`), the
//! DAG builder (`dag`), content-hash invalidation (`proven`), and the
//! cycle-safe reachability walk (`graph::transitive_imports`) — consumes
//! `&dyn Backend` and inherits cycle safety for free.

use crate::lint::{LintRule, RuleConfig};
use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

pub mod agda;
pub mod idris2;
pub mod isabelle;
pub mod lean;
pub mod mizar;
pub mod rocq;
pub mod smt;

pub use agda::{Agda, AgdaCubical};
pub use idris2::Idris2;
pub use isabelle::Isabelle;
pub use lean::Lean;
pub use mizar::Mizar;
pub use rocq::Coq;
pub use smt::Smt;

/// The two backend interaction models.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// A proof assistant: typecheck a source file; the exit code is the
    /// verdict (Agda, Idris2, Lean4, Coq/Rocq, Isabelle, Mizar).
    Assistant,
    /// A solver: feed a query and parse `sat`/`unsat`/`unknown` (Z3, CVC5).
    Solver,
}

/// The common verdict both interaction models map into.
///
/// Only [`Verdict::Proven`] is "green". `Admitted`/`Postulated` are amber —
/// a goal *stated* but not discharged — and must never be counted as proof
/// (this is exactly the silent-failure class the linter and the estate's
/// proof-drift work name). `Refuted`/`Unknown` are the solver outcomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Assistant exited 0, or solver returned `unsat` (VC discharged).
    Proven,
    /// Solver returned `sat` — a counterexample/model exists.
    Refuted,
    /// Solver returned `unknown`, or the run timed out.
    Unknown,
    /// The tool ran, but the artefact contains an admitted goal.
    Admitted,
    /// The tool ran, but the artefact contains a postulate/axiom.
    Postulated,
    /// The tool ran and errored (type error, parse error, …).
    Error,
    /// The tool was not found / could not be executed.
    Unavailable,
}

/// The result of a [`Backend::check_file`] invocation.
///
/// A superset of the old `AgdaOutcome`: it keeps the raw process facts
/// (`available`, `exit_code`, `ok`, `output_tail`) and adds the backend
/// `kind` plus the mapped [`Verdict`], so a caller sees both what the tool
/// literally did and how arghda classifies it.
#[derive(Clone, Debug, Serialize)]
pub struct Outcome {
    /// Whether the tool binary was found and executed.
    pub available: bool,
    /// Process exit code, if the process ran.
    pub exit_code: Option<i32>,
    /// `true` iff the tool exited 0 (assistant) / returned `unsat` (solver).
    pub ok: bool,
    /// Last few lines of combined stdout+stderr (for surfacing errors).
    pub output_tail: String,
    /// Which interaction model produced this outcome.
    pub kind: BackendKind,
    /// The verdict derived *only* from the tool's actual output.
    pub verdict: Verdict,
}

impl Outcome {
    /// The tool was not found on `PATH`; the graceful-degradation form.
    pub fn unavailable(kind: BackendKind) -> Self {
        Self {
            available: false,
            exit_code: None,
            ok: false,
            output_tail: String::new(),
            kind,
            verdict: Verdict::Unavailable,
        }
    }
}

/// The result of probing whether a backend's tool is actually runnable —
/// the honest input to `arghda doctor`.
#[derive(Clone, Debug, Serialize)]
pub struct Probe {
    /// The backend id (`agda`, `z3`, …).
    pub backend: String,
    /// Assistant or solver.
    pub kind: BackendKind,
    /// `true` iff the tool's binary could be executed.
    pub runnable: bool,
    /// The tool's version line, or why it is not runnable.
    pub detail: String,
}

/// Probe a tool by running `<cmd> --version`. Reports `runnable` honestly:
/// `false` only when the binary is absent / cannot be executed (never a
/// guess). `detail` is the first output line, or the reason it failed.
fn probe_tool(name: &str, kind: BackendKind, cmd: &str) -> Probe {
    probe_tool_arg(name, kind, cmd, "--version")
}

/// As [`probe_tool`], but with an explicit version-query argument — some
/// tools use a subcommand (`isabelle version`) rather than `--version`.
pub(crate) fn probe_tool_arg(name: &str, kind: BackendKind, cmd: &str, version_arg: &str) -> Probe {
    match Command::new(cmd).arg(version_arg).output() {
        Ok(out) => {
            let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&out.stderr));
            let first = combined.lines().next().unwrap_or("").trim().to_string();
            Probe {
                backend: name.to_string(),
                kind,
                runnable: true,
                detail: if first.is_empty() {
                    format!("ran `{cmd} --version`")
                } else {
                    first
                },
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Probe {
            backend: name.to_string(),
            kind,
            runnable: false,
            detail: format!("`{cmd}` not found on PATH"),
        },
        Err(e) => Probe {
            backend: name.to_string(),
            kind,
            runnable: false,
            detail: format!("`{cmd}` could not run: {e}"),
        },
    }
}

/// An object-safe adapter over one external prover or solver.
///
/// Exactly the seams that used to be hardcoded to Agda live behind this
/// trait: invocation ([`Backend::check_file`]), the import-graph trio
/// ([`Backend::module_name_of`] / [`Backend::module_to_path`] /
/// [`Backend::direct_imports`]), root discovery
/// ([`Backend::discover_roots`]), and the per-language lint pack
/// ([`Backend::lint_rules`]). Solvers legitimately return empty imports and
/// no roots — isolated DAG nodes are valid.
pub trait Backend: Send + Sync {
    /// Stable identifier (`"agda"`, `"idris2"`, `"z3"`, …).
    fn name(&self) -> &'static str;

    /// Assistant or solver.
    fn kind(&self) -> BackendKind;

    /// Source-file extensions this backend claims, without the dot
    /// (`["agda"]`, `["smt2"]`).
    fn extensions(&self) -> &'static [&'static str];

    /// The tool's safe/total mode flag, if any (for `doctor` / display).
    fn safe_mode(&self) -> Option<&'static str> {
        None
    }

    /// Run the tool on `file` and return its *actual* verdict. Must degrade
    /// to [`Outcome::unavailable`] when the tool is absent rather than
    /// erroring, so the rest of the engine still works tool-less.
    fn check_file(&self, file: &Path, include_root: &Path) -> Result<Outcome>;

    /// Dotted module name for `file` under `include_root`
    /// (`Ordinal/Closure.agda` → `Ordinal.Closure`). `None` if unresolvable.
    fn module_name_of(&self, file: &Path, include_root: &Path) -> Option<String>;

    /// Inverse of [`Backend::module_name_of`].
    fn module_to_path(&self, module: &str, include_root: &Path) -> PathBuf;

    /// The modules `file` imports. In-tree resolution is the caller's job;
    /// a solver with no import notion returns an empty vec.
    fn direct_imports(&self, file: &Path) -> Result<Vec<String>>;

    /// Conventional CI entry modules under `include_root` (e.g. Agda's
    /// `All.agda` / `Smoke.agda`). May be empty.
    fn discover_roots(&self, include_root: &Path) -> Vec<PathBuf>;

    /// The per-language lint pack, parameterised by operator config.
    fn lint_rules(&self, cfg: &RuleConfig) -> Result<Vec<Box<dyn LintRule>>>;

    /// The external binary this backend shells out to (`agda`, `z3`, …).
    /// May differ from [`Backend::name`] — e.g. `agda-cubical` runs `agda`.
    fn command(&self) -> &'static str;

    /// Probe whether the tool is actually runnable (for `arghda doctor`).
    /// The default runs `<command> --version`; honest — `runnable` is false
    /// only when the binary genuinely cannot be executed.
    fn probe(&self) -> Probe {
        probe_tool(self.name(), self.kind(), self.command())
    }
}

/// The default backend when none is selected: Agda, the v0.1 language.
pub fn default_backend() -> Box<dyn Backend> {
    Box::new(Agda)
}
