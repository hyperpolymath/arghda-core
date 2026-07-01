// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! arghda-core: proof-workspace manager for provers and solvers.
//!
//! Agda is the first (v0.1 reference) backend; every prover-specific seam
//! lives behind the [`prover::Backend`] trait, so the four-state workspace,
//! DAG builder and content-hash invalidation are backend-neutral. Public
//! surface is intentionally small: `Workspace`, the `Backend` trait, the
//! lint traits, and the diagnostic types. The CLI in `main.rs` is a thin
//! consumer.

pub mod config;
pub mod dag;
pub mod diagnostic;
pub mod event;
pub mod graph;
pub mod hash;
pub mod lint;
pub mod proven;
pub mod prover;
pub mod reason;
pub mod timestamp;
pub mod unused;
pub mod watcher;
pub mod workspace;

pub use dag::{build as build_dag, DagDocument};
pub use diagnostic::{Diagnostic, LintReport, Severity};
pub use event::{Event, EventKind};
pub use graph::{build as build_graph, ImportGraph};
pub use lint::{default_rules, rules_with_config, run_lints, LintRule, RuleConfig};
pub use prover::{default_backend, Agda, Backend, BackendKind, Idris2, Outcome, Smt, Verdict};
pub use reason::{build as build_reason, Junct, ReasonDocument, ReasonEdge, ReasonNode};
pub use workspace::{StaleEntry, State, Workspace};
