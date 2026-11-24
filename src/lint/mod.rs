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

/// Blank out Haskell-style block comments `{- … -}` (Agda, Idris2), which
/// nest, so that text inside them is not mistaken for code by the
/// line-based lint rules. Every character inside a comment becomes a space
/// except newlines, so line numbers and columns are unchanged. Agda pragmas
/// `{-# … #-}` are *not* comments and are kept verbatim: escape hatches such
/// as `{-# TERMINATING #-}` live in them. A comment left unclosed at end of
/// file blanks the rest of the file, as the compiler would treat it.
pub fn blank_block_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut depth = 0usize;
    let mut i = 0;
    while i < b.len() {
        if depth == 0 && b[i..].starts_with(b"{-#") {
            // A pragma: copy through to its closing `#-}`.
            let end = src[i..].find("#-}").map_or(src.len(), |e| i + e + 3);
            out.push_str(&src[i..end]);
            i = end;
        } else if b[i..].starts_with(b"{-") {
            depth += 1;
            out.push_str("  ");
            i += 2;
        } else if depth > 0 && b[i..].starts_with(b"-}") {
            depth -= 1;
            out.push_str("  ");
            i += 2;
        } else {
            let ch = src[i..].chars().next().expect("in bounds");
            if depth > 0 && ch != '\n' {
                out.push(' ');
            } else {
                out.push(ch);
            }
            i += ch.len_utf8();
        }
    }
    out
}

#[cfg(test)]
mod block_comment_tests {
    use super::blank_block_comments;

    #[test]
    fn single_line_comment_is_blanked() {
        assert_eq!(blank_block_comments("a {- b -} c"), "a         c");
    }

    #[test]
    fn multi_line_comment_keeps_newlines() {
        let out = blank_block_comments("x\n{- believe_me\n   here -}\ny");
        assert_eq!(out.lines().count(), 4);
        assert!(!out.contains("believe_me"));
        assert!(out.ends_with("\ny"));
    }

    #[test]
    fn nested_comment_is_blanked_to_its_outer_close() {
        let out = blank_block_comments("{- a {- b -} believe_me -} z");
        assert!(!out.contains("believe_me"));
        assert!(out.ends_with(" z"));
    }

    #[test]
    fn pragma_is_kept_verbatim() {
        let src = "{-# TERMINATING #-}\nf = f";
        assert_eq!(blank_block_comments(src), src);
    }

    #[test]
    fn unclosed_comment_blanks_the_rest() {
        let out = blank_block_comments("ok\n{- believe_me");
        assert!(out.starts_with("ok\n"));
        assert!(!out.contains("believe_me"));
    }

    #[test]
    fn multibyte_text_is_preserved_outside_comments() {
        assert_eq!(blank_block_comments("λ {- ∀ -} →"), "λ         →");
    }
}
