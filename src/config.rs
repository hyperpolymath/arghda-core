// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Operator configuration from `.arghda/config.toml`.
//!
//! The spec makes the `unpinned-headline` pattern operator-overridable
//! per-workspace (`docs/arghda-spec.adoc` §Open questions). This module loads
//! that override (and any future knobs) from a source tree's or workspace's
//! `.arghda/config.toml`:
//!
//! ```toml
//! [lint]
//! headline_pattern = "^[a-z][A-Za-z0-9-]*$"
//! ```
//!
//! Precedence (low → high): built-in [`RuleConfig::default`] < `config.toml`
//! < CLI flag (e.g. `--headline-pattern`). A missing file is not an error —
//! defaults apply. Unknown keys are rejected so a typo surfaces rather than
//! being silently ignored.

use crate::lint::RuleConfig;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Conventional config location, relative to a source tree or workspace root.
pub const CONFIG_REL_PATH: &str = ".arghda/config.toml";

/// The on-disk `.arghda/config.toml` shape. Mirror of the knobs in
/// [`RuleConfig`], all optional so a partial file overlays defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    lint: LintTable,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LintTable {
    /// Override for the `unpinned-headline` detection regex.
    headline_pattern: Option<String>,
}

/// Parse config TOML text into a [`RuleConfig`], overlaying built-in defaults.
fn parse(text: &str) -> Result<RuleConfig> {
    let file: ConfigFile = toml::from_str(text).context("parsing .arghda/config.toml")?;
    let mut cfg = RuleConfig::default();
    if let Some(p) = file.lint.headline_pattern {
        cfg.headline_pattern = p;
    }
    Ok(cfg)
}

/// Load a config file expected to exist, overlaying defaults.
pub fn load_file(path: &Path) -> Result<RuleConfig> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading config {}", path.display()))?;
    parse(&text).with_context(|| format!("in config {}", path.display()))
}

/// Load `<base>/.arghda/config.toml` if it exists, else built-in defaults.
pub fn load_from_dir(base: &Path) -> Result<RuleConfig> {
    let candidate = base.join(CONFIG_REL_PATH);
    if candidate.is_file() {
        load_file(&candidate)
    } else {
        Ok(RuleConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::unpinned_headline::DEFAULT_HEADLINE_PATTERN;

    #[test]
    fn lint_table_overrides_headline_pattern() {
        let cfg = parse("[lint]\nheadline_pattern = \"^thm-.*$\"\n").unwrap();
        assert_eq!(cfg.headline_pattern, "^thm-.*$");
    }

    #[test]
    fn empty_or_partial_file_keeps_defaults() {
        assert_eq!(
            parse("").unwrap().headline_pattern,
            DEFAULT_HEADLINE_PATTERN
        );
        assert_eq!(
            parse("[lint]\n").unwrap().headline_pattern,
            DEFAULT_HEADLINE_PATTERN
        );
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(parse("[lint]\nheadline_patten = \"x\"\n").is_err()); // typo
        assert!(parse("[bogus]\nx = 1\n").is_err()); // unknown table
    }

    #[test]
    fn malformed_toml_errors() {
        assert!(parse("[lint").is_err());
    }

    #[test]
    fn load_from_dir_without_config_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_from_dir(dir.path()).unwrap();
        assert_eq!(cfg.headline_pattern, DEFAULT_HEADLINE_PATTERN);
    }

    #[test]
    fn load_from_dir_reads_present_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".arghda")).unwrap();
        std::fs::write(
            dir.path().join(CONFIG_REL_PATH),
            "[lint]\nheadline_pattern = \"^[A-Z].*$\"\n",
        )
        .unwrap();
        let cfg = load_from_dir(dir.path()).unwrap();
        assert_eq!(cfg.headline_pattern, "^[A-Z].*$");
    }
}
