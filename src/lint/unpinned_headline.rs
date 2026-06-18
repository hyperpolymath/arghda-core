//! `unpinned-headline` (warn) — a module declares a top-level theorem whose
//! name matches the headline pattern, but the name is not pinned in any
//! `Smoke.agda` via a `using ( … )` clause.
//!
//! The estate discipline (echo-types' CLAUDE.md "Working rules": *every
//! headline theorem must be pinned in `Smoke.agda` via a `using` clause*)
//! is what keeps a `--safe` suite honest — a renamed or silently-dropped
//! headline surfaces as a broken pin rather than a quiet regression. This
//! rule makes the *missing* pin visible. It is a `warn`, not a hard-block:
//! not every top-level definition matching the pattern is a headline the
//! operator wants pinned, and the pattern is operator-configurable.
//!
//! Headline detection: top-level (column-0) type signatures `name : T`
//! (or `a b c : T`, which declares each of `a`, `b`, `c`) whose name
//! matches the configured regex. The column-0-only rule gives the
//! "export-only" filter for free — `private`-block definitions are indented
//! and so are never considered.
//!
//! Pins: the union of every `using ( … )` list across the `Smoke.agda`
//! files among the workspace's entry modules. If no `Smoke.agda` is in
//! scope (e.g. a single-file `check`), the rule self-skips rather than
//! flagging every headline.

use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use anyhow::{Context, Result};
use regex::Regex;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Spec default (`docs/arghda-spec.adoc` §Open questions): a lowercase-initial
/// ASCII identifier, hyphens allowed, fully anchored.
pub const DEFAULT_HEADLINE_PATTERN: &str = r"^[a-z][A-Za-z0-9-]*$";

/// Reserved openers that begin a top-level form but never declare a
/// pinnable headline name (so a `record … : Set where` line is not read
/// as a theorem signature).
const NON_DECL_KEYWORDS: &[&str] = &[
    "module",
    "open",
    "import",
    "private",
    "record",
    "data",
    "postulate",
    "infix",
    "infixl",
    "infixr",
    "mutual",
    "abstract",
    "instance",
    "field",
    "syntax",
    "pattern",
    "variable",
    "primitive",
    "where",
    "renaming",
    "using",
    "hiding",
    "constructor",
    "interleaved",
    "unquoteDecl",
    "unquoteDef",
    "macro",
    "tactic",
];

pub struct UnpinnedHeadline {
    matcher: Regex,
}

impl UnpinnedHeadline {
    /// Build the rule with an operator-supplied headline pattern. Returns an
    /// error if the pattern is not a valid regex.
    pub fn new(pattern: &str) -> Result<Self> {
        let matcher = Regex::new(pattern)
            .with_context(|| format!("compiling headline pattern `{pattern}`"))?;
        Ok(Self { matcher })
    }
}

impl Default for UnpinnedHeadline {
    fn default() -> Self {
        Self::new(DEFAULT_HEADLINE_PATTERN).expect("default headline pattern is a valid regex")
    }
}

impl LintRule for UnpinnedHeadline {
    fn name(&self) -> &'static str {
        "unpinned-headline"
    }

    fn run(&self, file: &Path, ctx: &LintContext<'_>, report: &mut LintReport) -> Result<()> {
        // The Smoke files are the pin registry, not theorem modules; never
        // lint them for their own headlines.
        if is_smoke(file) {
            return Ok(());
        }

        // Union the pin set across every Smoke.agda among the entry modules.
        let mut pinned: BTreeSet<String> = BTreeSet::new();
        let mut any_smoke = false;
        for entry in ctx.entry_modules {
            if !is_smoke(entry) {
                continue;
            }
            any_smoke = true;
            let contents = fs::read_to_string(entry)
                .with_context(|| format!("reading Smoke file {}", entry.display()))?;
            collect_pinned(&contents, &mut pinned);
        }
        if !any_smoke {
            // Nothing pins anything in scope; can't judge pinning.
            return Ok(());
        }

        let contents =
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;

        let mut reported: BTreeSet<String> = BTreeSet::new();
        for (name, line) in headline_decls(&contents, &self.matcher) {
            if pinned.contains(&name) || !reported.insert(name.clone()) {
                continue;
            }
            report.push(Diagnostic {
                rule: self.name().to_string(),
                severity: Severity::Warn,
                file: file.to_path_buf(),
                message: format!(
                    "headline `{name}` is not pinned in any Smoke.agda via a `using` clause"
                ),
                line: Some(line),
            });
        }
        Ok(())
    }
}

fn is_smoke(path: &Path) -> bool {
    path.file_name().and_then(|s| s.to_str()) == Some("Smoke.agda")
}

/// Strip an Agda line comment for token analysis: a whole-line `--` comment
/// becomes empty; an inline ` --` comment is cut. Mirrors the convention the
/// `escape-hatch` rule uses.
fn strip_comment(line: &str) -> &str {
    if line.trim_start().starts_with("--") {
        return "";
    }
    line.split(" --").next().unwrap_or(line)
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '\''
}

/// Collect the names appearing in `using ( … )` clauses of `contents` into
/// `out`. Tolerant of multi-line `using` lists; entries are `;`-separated.
fn collect_pinned(contents: &str, out: &mut BTreeSet<String>) {
    // Strip comments first so a `-- … using (…)` header does not register.
    let cleaned: String = contents
        .lines()
        .map(strip_comment)
        .collect::<Vec<_>>()
        .join("\n");

    let mut i = 0usize;
    while let Some(rel) = cleaned[i..].find("using") {
        let start = i + rel;
        let end = start + "using".len();
        i = end;

        // Left word boundary: the char before `using` must not be part of a
        // longer identifier (so `reusing` does not match).
        let left_ok = start == 0 || !is_ident_char(cleaned[..start].chars().next_back().unwrap());
        if !left_ok {
            continue;
        }

        // Skip whitespace, then require `(`. (This also enforces the right
        // word boundary: `usingFoo` has no `(` after `using`.)
        let rest = &cleaned[end..];
        let mut off = 0usize;
        for c in rest.chars() {
            if c.is_whitespace() {
                off += c.len_utf8();
            } else {
                break;
            }
        }
        if !rest[off..].starts_with('(') {
            continue;
        }

        let inner_start = off + 1;
        let Some(close_rel) = rest[inner_start..].find(')') else {
            continue;
        };
        let inner = &rest[inner_start..inner_start + close_rel];
        for entry in inner.split(';') {
            let n = entry.trim();
            if !n.is_empty() {
                out.insert(n.to_string());
            }
        }
        i = end + inner_start + close_rel + 1;
    }
}

/// Top-level (column-0) type-signature names matching `matcher`, paired with
/// the 1-based line they were declared on. `a b c : T` yields each name.
fn headline_decls(contents: &str, matcher: &Regex) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut in_block_comment = false;

    for (idx, raw) in contents.lines().enumerate() {
        if in_block_comment {
            if raw.contains("-}") {
                in_block_comment = false;
            }
            continue;
        }
        // Only column-0 lines are top-level (export-only filter): an indented
        // line is inside a module / record / private block.
        if raw.is_empty() || raw.starts_with(char::is_whitespace) {
            continue;
        }
        // Whole-line comment.
        if raw.starts_with("--") {
            continue;
        }
        // Pragma (`{-# … #-}`) or block-comment opener (`{- …`). Handled
        // before comment-stripping so an OPTIONS pragma carrying `--safe`
        // is never mistaken for a `--` comment.
        if raw.starts_with("{-") {
            if !raw.contains("-}") {
                in_block_comment = true;
            }
            continue;
        }

        let line = strip_comment(raw);
        let tokens: Vec<&str> = line.split_whitespace().collect();
        // Need the `name … :` shape: a standalone `:` token after ≥1 name.
        let Some(colon) = tokens.iter().position(|&t| t == ":") else {
            continue;
        };
        if colon == 0 || NON_DECL_KEYWORDS.contains(&tokens[0]) {
            continue;
        }
        for name in &tokens[..colon] {
            if matcher.is_match(name) {
                out.push(((*name).to_string(), idx + 1));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::run_lints;
    use std::path::PathBuf;

    /// Lint `module` (written as `Foo.agda`) for unpinned headlines, with an
    /// optional sibling `Smoke.agda` registered as the sole entry module.
    fn run_rule(module: &str, smoke: Option<&str>, pattern: &str) -> LintReport {
        let dir = tempfile::tempdir().unwrap();
        let modpath = dir.path().join("Foo.agda");
        std::fs::write(&modpath, module).unwrap();
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(s) = smoke {
            let sp = dir.path().join("Smoke.agda");
            std::fs::write(&sp, s).unwrap();
            roots.push(sp);
        }
        let ctx = LintContext {
            include_root: dir.path(),
            entry_modules: &roots,
        };
        let rules: Vec<Box<dyn LintRule>> = vec![Box::new(UnpinnedHeadline::new(pattern).unwrap())];
        run_lints(&modpath, &ctx, &rules).unwrap()
    }

    #[test]
    fn unpinned_headline_is_warned() {
        let r = run_rule(
            "module Foo where\nfoo-bar : Set\nfoo-bar = Set\n",
            Some("module Smoke where\nopen import Bar using ( something-else )\n"),
            DEFAULT_HEADLINE_PATTERN,
        );
        assert!(!r.has_hard_block());
        assert_eq!(r.warns().count(), 1, "got: {:?}", r.diagnostics);
        assert!(r.diagnostics[0].message.contains("foo-bar"));
        assert_eq!(r.diagnostics[0].line, Some(2));
    }

    #[test]
    fn pinned_headline_is_clean() {
        let r = run_rule(
            "module Foo where\nfoo-bar : Set\nfoo-bar = Set\n",
            Some("module Smoke where\nopen import Foo using ( foo-bar )\n"),
            DEFAULT_HEADLINE_PATTERN,
        );
        assert!(r.diagnostics.is_empty(), "got: {:?}", r.diagnostics);
    }

    #[test]
    fn multi_name_signature_flags_only_unpinned() {
        let r = run_rule(
            "module Foo where\nlemma-a lemma-b : Set\n",
            Some("module Smoke where\nopen import Foo using ( lemma-a )\n"),
            DEFAULT_HEADLINE_PATTERN,
        );
        assert_eq!(r.warns().count(), 1, "got: {:?}", r.diagnostics);
        assert!(r.diagnostics[0].message.contains("lemma-b"));
    }

    #[test]
    fn private_indented_decl_is_not_a_headline() {
        // `helper-fn` is indented under `private` → not column-0 → ignored.
        let r = run_rule(
            "module Foo where\nprivate\n  helper-fn : Set\n",
            Some("module Smoke where\nopen import Bar using ( x )\n"),
            DEFAULT_HEADLINE_PATTERN,
        );
        assert!(r.diagnostics.is_empty(), "got: {:?}", r.diagnostics);
    }

    #[test]
    fn uppercase_name_is_not_matched_by_default_pattern() {
        let r = run_rule(
            "module Foo where\nC-monotone : Set\n",
            Some("module Smoke where\nopen import Bar using ( x )\n"),
            DEFAULT_HEADLINE_PATTERN,
        );
        assert!(r.diagnostics.is_empty(), "got: {:?}", r.diagnostics);
    }

    #[test]
    fn custom_pattern_can_match_uppercase() {
        let r = run_rule(
            "module Foo where\nC-monotone : Set\n",
            Some("module Smoke where\nopen import Bar using ( x )\n"),
            r"^[A-Za-z][A-Za-z0-9-]*$",
        );
        assert_eq!(r.warns().count(), 1, "got: {:?}", r.diagnostics);
        assert!(r.diagnostics[0].message.contains("C-monotone"));
    }

    #[test]
    fn no_smoke_in_scope_skips() {
        let r = run_rule(
            "module Foo where\nfoo-bar : Set\n",
            None,
            DEFAULT_HEADLINE_PATTERN,
        );
        assert!(r.diagnostics.is_empty(), "got: {:?}", r.diagnostics);
    }

    #[test]
    fn multiline_using_list_pins() {
        let r = run_rule(
            "module Foo where\nfoo-bar : Set\nbaz-qux : Set\n",
            Some("module Smoke where\nopen import Foo using\n  ( foo-bar\n  ; baz-qux\n  )\n"),
            DEFAULT_HEADLINE_PATTERN,
        );
        assert!(r.diagnostics.is_empty(), "got: {:?}", r.diagnostics);
    }

    #[test]
    fn options_pragma_is_not_read_as_a_decl() {
        // The `--safe` in the pragma must not derail signature detection on
        // the following line.
        let r = run_rule(
            "{-# OPTIONS --safe --without-K #-}\nmodule Foo where\nfoo-bar : Set\n",
            Some("module Smoke where\nopen import Foo using ( foo-bar )\n"),
            DEFAULT_HEADLINE_PATTERN,
        );
        assert!(r.diagnostics.is_empty(), "got: {:?}", r.diagnostics);
    }

    #[test]
    fn smoke_file_itself_is_skipped() {
        // A file *named* Smoke.agda is never linted for headlines, even if it
        // carries a signature.
        let dir = tempfile::tempdir().unwrap();
        let smoke = dir.path().join("Smoke.agda");
        std::fs::write(&smoke, "module Smoke where\nlocal-lemma : Set\n").unwrap();
        let roots = [smoke.clone()];
        let ctx = LintContext {
            include_root: dir.path(),
            entry_modules: &roots,
        };
        let rules: Vec<Box<dyn LintRule>> = vec![Box::new(
            UnpinnedHeadline::new(DEFAULT_HEADLINE_PATTERN).unwrap(),
        )];
        let r = run_lints(&smoke, &ctx, &rules).unwrap();
        assert!(r.diagnostics.is_empty(), "got: {:?}", r.diagnostics);
    }

    #[test]
    fn invalid_pattern_is_an_error() {
        assert!(UnpinnedHeadline::new("[").is_err());
    }

    #[test]
    fn record_signature_is_not_a_headline() {
        // `record Foo : Set where` has a `:` but opens a record, not a theorem.
        let r = run_rule(
            "module Foo where\nrecord Wrap : Set where\n",
            Some("module Smoke where\nopen import Bar using ( x )\n"),
            DEFAULT_HEADLINE_PATTERN,
        );
        assert!(r.diagnostics.is_empty(), "got: {:?}", r.diagnostics);
    }
}
