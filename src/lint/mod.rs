// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

use crate::diagnostic::LintReport;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub mod coq;
pub mod escape_hatch;
pub mod idris2;
pub mod isabelle;
pub mod lean;
pub mod orphan_module;
pub mod postulate;
pub mod safe_pragma;
pub mod tab_mix;
pub mod unpinned_headline;

/// Context handed to every rule.
#[derive(Clone, Debug)]
pub struct LintContext<'a> {
    /// Agda include root; `.agda` files' module names are computed
    /// relative to this path.
    pub include_root: &'a Path,
    /// The root modules (e.g. `All.agda`, `Smoke.agda`). Reachability is
    /// computed from the *union* of these, so a module verified from any
    /// CI entry point is not an orphan.
    pub entry_modules: &'a [PathBuf],
}

pub trait LintRule: Send + Sync {
    fn name(&self) -> &'static str;
    fn run(&self, file: &Path, ctx: &LintContext<'_>, report: &mut LintReport) -> Result<()>;
}

/// Operator-configurable lint settings (the `.arghda/config.toml` surface
/// the spec calls for; currently set via the CLI `--headline-pattern` flag).
#[derive(Clone, Debug)]
pub struct RuleConfig {
    /// Regex a top-level definition name must match to be treated as a
    /// pinnable headline by the `unpinned-headline` rule.
    pub headline_pattern: String,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            headline_pattern: unpinned_headline::DEFAULT_HEADLINE_PATTERN.to_string(),
        }
    }
}

/// The Agda lint pack around a given `missing-safe-pragma` profile.
fn agda_pack(cfg: &RuleConfig, safe: safe_pragma::SafePragma) -> Result<Vec<Box<dyn LintRule>>> {
    Ok(vec![
        Box::new(safe),
        Box::new(orphan_module::OrphanModule),
        Box::new(postulate::UnjustifiedPostulate),
        Box::new(escape_hatch::EscapeHatch),
        Box::new(tab_mix::TabMix),
        Box::new(unpinned_headline::UnpinnedHeadline::new(
            &cfg.headline_pattern,
        )?),
    ])
}

/// The standard `--safe --without-K` Agda lint pack, parameterised by
/// operator config. Fails only if a supplied pattern (e.g.
/// `headline_pattern`) is not a valid regex.
pub fn rules_with_config(cfg: &RuleConfig) -> Result<Vec<Box<dyn LintRule>>> {
    agda_pack(cfg, safe_pragma::SafePragma::standard())
}

/// The Cubical-Agda (`--cubical --safe`) lint pack — identical to the
/// standard pack except the `missing-safe-pragma` rule requires `--cubical`
/// instead of `--without-K`.
pub fn agda_cubical_rules(cfg: &RuleConfig) -> Result<Vec<Box<dyn LintRule>>> {
    agda_pack(cfg, safe_pragma::SafePragma::cubical())
}

/// The standard lint pack with default config. The default pattern is a
/// known-good literal, so this is infallible.
pub fn default_rules() -> Vec<Box<dyn LintRule>> {
    rules_with_config(&RuleConfig::default()).expect("default rule config is valid")
}

pub fn run_lints(
    file: &Path,
    ctx: &LintContext<'_>,
    rules: &[Box<dyn LintRule>],
) -> Result<LintReport> {
    let mut report = LintReport::new(file.to_path_buf());
    for rule in rules {
        rule.run(file, ctx, &mut report)?;
    }
    Ok(report)
}
