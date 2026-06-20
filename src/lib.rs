//! arghda-core: proof-workspace manager for Agda.
//!
//! Public surface is intentionally small: `Workspace`, the lint traits,
//! and the diagnostic types. The CLI in `main.rs` is a thin consumer.

pub mod agda;
pub mod config;
pub mod dag;
pub mod diagnostic;
pub mod event;
pub mod graph;
pub mod hash;
pub mod lint;
pub mod proven;
pub mod timestamp;
pub mod unused;
pub mod watcher;
pub mod workspace;

pub use agda::{check_file, AgdaOutcome};
pub use dag::{build as build_dag, DagDocument};
pub use diagnostic::{Diagnostic, LintReport, Severity};
pub use event::{Event, EventKind};
pub use graph::{build as build_graph, ImportGraph};
pub use lint::{default_rules, rules_with_config, run_lints, LintRule, RuleConfig};
pub use workspace::{StaleEntry, State, Workspace};
