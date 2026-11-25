// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

use crate::diagnostic::LintReport;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub mod coq;
pub mod escape_hatch;
pub mod idris2;
pub mod idris2_lex;
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

/// Blank out Agda block comments `{- … -}`, which nest, so that text inside
/// them is not mistaken for code by the line-based lint rules. (Idris2 lexes
/// comments and strings differently and uses [`idris2_lex::code_view`].) Every character inside a comment becomes a space
/// except newlines, so line numbers are unchanged.
///
/// Fail-closed by construction: text is blanked only when it is certainly
/// inside a closed block comment. Anything uncertain is kept verbatim, so a
/// lint can over-report but never miss code:
///
/// * a `{-` inside a `--` line comment or a `"…"` string literal does not
///   open a comment (both are copied through unchanged);
/// * Agda pragmas `{-# … #-}` are kept verbatim, since escape hatches such as
///   `{-# TERMINATING #-}` live in them;
/// * a `{-` with no matching `-}` is left as is, together with the rest of
///   the file (the compiler rejects such a file anyway).
pub fn blank_block_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        let rest = &b[i..];
        if rest.starts_with(b"{-#") {
            // A pragma: copy through to its closing `#-}`.
            let end = src[i..].find("#-}").map_or(src.len(), |e| i + e + 3);
            out.push_str(&src[i..end]);
            i = end;
        } else if rest.starts_with(b"--") {
            // A line comment: copy verbatim to the end of the line.
            let end = src[i..].find('\n').map_or(src.len(), |e| i + e);
            out.push_str(&src[i..end]);
            i = end;
        } else if rest.starts_with(b"\"") {
            // A string literal: copy verbatim to its unescaped closing quote,
            // or to the end of the line if it is not closed there.
            let mut j = i + 1;
            while j < b.len() && b[j] != b'\n' {
                if b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if b[j] == b'"' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            let end = j.min(b.len());
            out.push_str(&src[i..end]);
            i = end;
        } else if rest.starts_with(b"{-") {
            match block_comment_end(b, i) {
                Some(end) => {
                    for ch in src[i..end].chars() {
                        out.push(if ch == '\n' { '\n' } else { ' ' });
                    }
                    i = end;
                }
                None => {
                    // Unclosed: keep the rest verbatim (fail-closed).
                    out.push_str(&src[i..]);
                    i = b.len();
                }
            }
        } else {
            let ch = src[i..].chars().next().expect("in bounds");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// The part of an Agda `line` before a trailing ` --` line comment, ignoring
/// any ` --` inside a `"…"` string literal (Codex review on #66: a string
/// holding ` --` used to cut the line before the code that followed it).
pub fn code_before_line_comment(line: &str) -> &str {
    let b = line.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        if in_str {
            match b[i] {
                b'\\' => i += 1,
                b'"' => in_str = false,
                _ => {}
            }
        } else if b[i] == b'"' {
            in_str = true;
        } else if b[i..].starts_with(b" --") {
            return &line[..i];
        }
        i += 1;
    }
    line
}

/// Byte index just past the `-}` that closes the block comment opened at
/// `start` (which must point at `{-`), honouring nesting; `None` if the
/// comment is never closed.
fn block_comment_end(b: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start;
    while i + 1 < b.len() {
        if b[i] == b'{' && b[i + 1] == b'-' {
            depth += 1;
            i += 2;
        } else if b[i] == b'-' && b[i + 1] == b'}' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Some(i);
            }
        } else {
            i += 1;
        }
    }
    None
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
    fn unclosed_comment_is_kept_verbatim() {
        // Fail-closed: nothing is hidden when the comment never closes.
        let src = "ok\n{- believe_me";
        assert_eq!(blank_block_comments(src), src);
    }

    #[test]
    fn open_brace_dash_in_a_line_comment_does_not_open_a_comment() {
        // Codex review on #66 (P1): this used to hide the code below.
        let src = "-- mention {- here\nx = believe_me 0\n";
        assert_eq!(blank_block_comments(src), src);
    }

    #[test]
    fn open_brace_dash_in_a_string_does_not_open_a_comment() {
        let src = "note = \"{-\"\nx = believe_me 0\n";
        assert_eq!(blank_block_comments(src), src);
        let esc = "s = \"a\\\"{-\"\nx = believe_me 0\n";
        assert_eq!(blank_block_comments(esc), esc);
    }

    #[test]
    fn comment_after_a_string_on_the_same_line_is_still_blanked() {
        let out = blank_block_comments("s = \"x\" {- believe_me -} y");
        assert!(!out.contains("believe_me"));
        assert!(out.starts_with("s = \"x\" "));
        assert!(out.ends_with(" y"));
    }

    #[test]
    fn multibyte_text_is_preserved_outside_comments() {
        assert_eq!(blank_block_comments("λ {- ∀ -} →"), "λ         →");
    }
}
