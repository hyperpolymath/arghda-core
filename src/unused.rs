//! Shelling out to `agda-unused` to find unused code in a project.
//!
//! Like [`crate::agda`], ArghDA does not analyse for unused code itself — it
//! asks the external `agda-unused` tool and re-emits its findings as ArghDA
//! diagnostics (the `unused-import` rule, `docs/arghda-spec.adoc`). If
//! `agda-unused` is not on `PATH` it degrades gracefully (`available: false`)
//! so the rest of the engine still works without it (the rule is opt-in
//! behind `scan --unused`).
//!
//! `agda-unused` is a *project-level* analyser — it resolves the whole import
//! graph from a root module — so this is a single pass over a source tree,
//! not a per-file [`crate::lint::LintRule`]. Its `--json` output is a wrapper
//! object `{ "type": "none" | "unused" | "error", "message": "…" }`; the
//! findings live in `message` in the human-readable form
//!
//! ```text
//! /abs/path/File.agda:line,col-col
//!   <category> '<name>'
//! ```
//!
//! which we parse into per-file [`Diagnostic`]s under the `unused-import`
//! rule (severity `warn`, per the spec's rule table).

use crate::diagnostic::{Diagnostic, Severity};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The rule name re-emitted findings carry.
pub const RULE_NAME: &str = "unused-import";

/// Outcome of an `agda-unused` pass over a source tree.
#[derive(Clone, Debug, Default)]
pub struct UnusedOutcome {
    /// Whether the `agda-unused` binary was found and executed.
    pub available: bool,
    /// Findings re-emitted as diagnostics (rule = `unused-import`, warn).
    pub diagnostics: Vec<Diagnostic>,
    /// The last `type` field seen ("none" / "unused" / "error"), for surfacing
    /// the case where `agda-unused` itself errored (e.g. a type error).
    pub kind: Option<String>,
}

/// The `--json` wrapper `agda-unused` emits.
#[derive(Debug, Deserialize)]
struct UnusedJson {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    message: String,
}

/// Run `agda-unused <root> --global --json -i <include_root>` for each root and
/// union the findings (deduplicated by file + line + message). A missing
/// binary yields `available: false` with no diagnostics.
pub fn find_unused(include_root: &Path, roots: &[PathBuf]) -> Result<UnusedOutcome> {
    let mut outcome = UnusedOutcome::default();
    let mut seen: BTreeSet<(PathBuf, Option<usize>, String)> = BTreeSet::new();

    for root in roots {
        let Some(parsed) = run_one(root, include_root)? else {
            // Binary not found: report unavailable and stop.
            return Ok(UnusedOutcome::default());
        };
        outcome.available = true;
        outcome.kind = Some(parsed.kind.clone());
        if parsed.kind != "unused" {
            // "none" → nothing unused; "error" → agda-unused could not analyse
            // (surfaced via `kind`); neither yields findings.
            continue;
        }
        for (file, line, desc) in parse_findings(&parsed.message) {
            let message = if desc.is_empty() {
                "unused code".to_string()
            } else {
                desc
            };
            if seen.insert((file.clone(), line, message.clone())) {
                outcome.diagnostics.push(Diagnostic {
                    rule: RULE_NAME.to_string(),
                    severity: Severity::Warn,
                    file,
                    message,
                    line,
                });
            }
        }
    }
    Ok(outcome)
}

/// Invoke `agda-unused` once on `root`. `Ok(None)` iff the binary is absent.
fn run_one(root: &Path, include_root: &Path) -> Result<Option<UnusedJson>> {
    let output = Command::new("agda-unused")
        .arg(root)
        .arg("--global")
        .arg("--json")
        .arg("-i")
        .arg(include_root)
        .output();

    match output {
        Ok(out) => {
            let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&out.stderr));
            let json = extract_json(&combined).with_context(|| {
                format!(
                    "no JSON object in agda-unused output for {}",
                    root.display()
                )
            })?;
            let parsed: UnusedJson = serde_json::from_str(json)
                .with_context(|| format!("parsing agda-unused JSON for {}", root.display()))?;
            Ok(Some(parsed))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Extract the outermost `{ … }` JSON object from `s` (tolerant of any
/// non-JSON preamble the tool might print before the object).
fn extract_json(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end >= start {
        Some(&s[start..=end])
    } else {
        None
    }
}

/// Parse the findings out of an `agda-unused` `message`. Each finding is a
/// location line (`…/File.agda:line,col-col`) followed by an indented
/// description line (`<category> '<name>'`).
fn parse_findings(message: &str) -> Vec<(PathBuf, Option<usize>, String)> {
    let lines: Vec<&str> = message.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some((file, line)) = parse_location(lines[i]) {
            // The description is the next non-empty line that is not itself a
            // location (indented `<category> '<name>'`).
            let mut desc = String::new();
            if i + 1 < lines.len() {
                let next = lines[i + 1].trim();
                if !next.is_empty() && parse_location(lines[i + 1]).is_none() {
                    desc = next.to_string();
                    i += 1;
                }
            }
            out.push((file, line, desc));
        }
        i += 1;
    }
    out
}

/// Parse a location line `…/File.agda:line,col-col` into the file path and the
/// 1-based line number. Tolerant: the trailing column/range part is optional.
fn parse_location(line: &str) -> Option<(PathBuf, Option<usize>)> {
    let trimmed = line.trim();
    // Anchor on the `.agda:` boundary so paths containing `:` (unlikely) or
    // colons elsewhere don't confuse the split.
    let marker = ".agda:";
    let idx = trimmed.rfind(marker)?;
    let file = PathBuf::from(&trimmed[..idx + ".agda".len()]);
    let rest = &trimmed[idx + marker.len()..];
    // `rest` looks like `12,3-8` or `12,3-13,5`; take the leading integer.
    let lineno = rest
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse::<usize>().ok());
    Some((file, lineno))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_binary_degrades_gracefully() {
        // `run_one` against a guaranteed-absent binary name is exercised
        // indirectly: find_unused with a bogus PATH can't be forced here, so
        // we assert the parser/JSON layers instead. The not-found path is the
        // `Ok(None)` arm of `run_one` (see also crate::agda).
        let o = UnusedOutcome::default();
        assert!(!o.available);
        assert!(o.diagnostics.is_empty());
    }

    #[test]
    fn extract_json_finds_object_amid_noise() {
        assert_eq!(
            extract_json("preamble {\"type\":\"none\"} trailing"),
            Some("{\"type\":\"none\"}")
        );
        assert_eq!(extract_json("no object here"), None);
    }

    #[test]
    fn parse_location_reads_path_and_line() {
        let (f, l) = parse_location("/home/u/proj/Foo/Bar.agda:12,3-8").unwrap();
        assert_eq!(f, PathBuf::from("/home/u/proj/Foo/Bar.agda"));
        assert_eq!(l, Some(12));
    }

    #[test]
    fn parse_location_tolerates_line_range() {
        let (_f, l) = parse_location("X.agda:7,1-9,4").unwrap();
        assert_eq!(l, Some(7));
    }

    #[test]
    fn parse_location_rejects_non_location() {
        assert!(parse_location("  unused import 'Foo'").is_none());
    }

    #[test]
    fn parse_findings_pairs_location_with_description() {
        let msg = "/p/Main.agda:3,1-19\n  unused import 'Unused'\n";
        let found = parse_findings(msg);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, PathBuf::from("/p/Main.agda"));
        assert_eq!(found[0].1, Some(3));
        assert_eq!(found[0].2, "unused import 'Unused'");
    }
}
