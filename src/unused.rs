// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Shelling out to `agda-unused` to find unused code (the `unused-import` rule).
//!
//! Like [`crate::agda`], ArghDA does not analyse for unused code itself — it
//! asks the external `agda-unused` tool and re-emits its findings as ArghDA
//! diagnostics (`docs/arghda-spec.adoc` §Linter rules). If `agda-unused` is
//! not on `PATH` it degrades gracefully (`available: false`) so the rest of
//! the engine still works; the rule is opt-in behind `scan --unused`.
//!
//! We invoke it **per file in local mode** (`agda-unused <file> --json -i
//! <root>`). Local mode flags imports/code unused *within* that file (the
//! rule's namesake); `--global` instead treats the given root's imports as
//! project roots and would not flag them. `agda-unused` reads source in the
//! process locale, so we force `LC_ALL=C.UTF-8` — Agda sources are UTF-8 and
//! without it the tool aborts on the first multi-byte character.
//!
//! `--json` output is a wrapper object
//! `{ "type": "none" | "unused" | "error", "message": "…" }`; when `type` is
//! `"unused"` the findings live in `message` as location/description pairs:
//!
//! ```text
//! /abs/path/File.agda:line,col-col
//!   unused import ‘Name’
//! ```
//!
//! re-emitted as `unused-import` `warn` diagnostics.

use crate::diagnostic::{Diagnostic, Severity};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The rule name re-emitted findings carry.
pub const RULE_NAME: &str = "unused-import";

/// Result of an `agda-unused` pass over a single file.
#[derive(Clone, Debug, Default)]
pub struct UnusedCheck {
    /// Whether the `agda-unused` binary was found and executed.
    pub available: bool,
    /// The `type` field of the JSON wrapper ("none" / "unused" / "error"),
    /// so the caller can surface the case where `agda-unused` could not
    /// analyse a file.
    pub kind: Option<String>,
    /// Findings located in the checked file, as `unused-import` warnings.
    pub diagnostics: Vec<Diagnostic>,
}

/// The `--json` wrapper `agda-unused` emits.
#[derive(Debug, Deserialize)]
struct UnusedJson {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    message: String,
}

/// Run `agda-unused <file> --json -i <include_root>` (local mode, UTF-8) and
/// return the unused-code findings located in `file`. `available: false` iff
/// the binary is absent.
pub fn check_file(file: &Path, include_root: &Path) -> Result<UnusedCheck> {
    let Some(parsed) = run_one(file, include_root)? else {
        return Ok(UnusedCheck::default());
    };
    let mut check = UnusedCheck {
        available: true,
        kind: Some(parsed.kind.clone()),
        diagnostics: Vec::new(),
    };
    if parsed.kind == "unused" {
        // Local mode can also visit dependencies; keep only findings that
        // belong to `file` so each is attributed exactly once (when we invoke
        // agda-unused on its own file). agda-unused reports absolute paths, so
        // compare canonicalised.
        let target = std::fs::canonicalize(file).ok();
        for (path, line, desc) in parse_findings(&parsed.message) {
            if target.is_some() && std::fs::canonicalize(&path).ok() == target {
                check.diagnostics.push(Diagnostic {
                    rule: RULE_NAME.to_string(),
                    severity: Severity::Warn,
                    file: file.to_path_buf(),
                    message: if desc.is_empty() {
                        "unused code".to_string()
                    } else {
                        desc
                    },
                    line,
                });
            }
        }
    }
    Ok(check)
}

/// Invoke `agda-unused` once on `file`. `Ok(None)` iff the binary is absent.
/// A run that produces no JSON object (e.g. the tool itself errored) is
/// reported as an `"error"` wrapper rather than failing the whole scan.
fn run_one(file: &Path, include_root: &Path) -> Result<Option<UnusedJson>> {
    let output = Command::new("agda-unused")
        .arg(file)
        .arg("--json")
        .arg("-i")
        .arg(include_root)
        .env("LC_ALL", "C.UTF-8")
        .output();

    match output {
        Ok(out) => {
            let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&out.stderr));
            match extract_json(&combined) {
                Some(json) => {
                    let parsed: UnusedJson = serde_json::from_str(json).with_context(|| {
                        format!("parsing agda-unused JSON for {}", file.display())
                    })?;
                    Ok(Some(parsed))
                }
                // No JSON: agda-unused crashed/errored on this file. Surface as
                // an error wrapper so one bad file does not abort the scan.
                None => Ok(Some(UnusedJson {
                    kind: "error".to_string(),
                    message: combined.trim().to_string(),
                })),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Extract the outermost `{ … }` JSON object from `s` (tolerant of any
/// non-JSON text printed around it).
fn extract_json(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    (end >= start).then(|| &s[start..=end])
}

/// Parse the findings out of an `agda-unused` `message`. Each finding is a
/// location line (`…/File.agda:line,col-col`) followed by an indented
/// description line (`unused import ‘Name’`).
fn parse_findings(message: &str) -> Vec<(PathBuf, Option<usize>, String)> {
    let lines: Vec<&str> = message.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some((file, line)) = parse_location(lines[i]) {
            // The description is the next non-empty line that is not itself a
            // location (the indented `<category> ‘<name>’`).
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
    // Anchor on the `.agda:` boundary so colons elsewhere don't confuse it.
    let marker = ".agda:";
    let idx = trimmed.rfind(marker)?;
    let file = PathBuf::from(&trimmed[..idx + ".agda".len()]);
    let rest = &trimmed[idx + marker.len()..];
    // `rest` looks like `5,1-19` or `5,1-9,4`; take the leading integer.
    let lineno = rest
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse::<usize>().ok());
    Some((file, lineno))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The wrapper agda-unused actually emits (captured live, local mode, on a
    // fixture with an unused import). Note the Unicode quotes ‘ ’.
    const REAL_UNUSED: &str =
        r#"{"message":"/p/Main.agda:5,1-19\n  unused import ‘Helper’","type":"unused"}"#;
    const REAL_NONE: &str = r#"{"message":"No unused code.","type":"none"}"#;

    #[test]
    fn default_outcome_is_unavailable_and_empty() {
        let c = UnusedCheck::default();
        assert!(!c.available);
        assert!(c.kind.is_none());
        assert!(c.diagnostics.is_empty());
    }

    #[test]
    fn extract_json_finds_object_amid_noise() {
        assert_eq!(
            extract_json("warn: blah {\"type\":\"none\"} trailing"),
            Some("{\"type\":\"none\"}")
        );
        assert_eq!(extract_json("no object here"), None);
    }

    #[test]
    fn deserialises_real_wrappers() {
        let unused: UnusedJson = serde_json::from_str(extract_json(REAL_UNUSED).unwrap()).unwrap();
        assert_eq!(unused.kind, "unused");
        assert!(unused.message.contains("unused import"));
        let none: UnusedJson = serde_json::from_str(extract_json(REAL_NONE).unwrap()).unwrap();
        assert_eq!(none.kind, "none");
    }

    #[test]
    fn parse_location_reads_path_and_line() {
        let (f, l) = parse_location("/home/u/proj/Foo/Bar.agda:12,3-8").unwrap();
        assert_eq!(f, PathBuf::from("/home/u/proj/Foo/Bar.agda"));
        assert_eq!(l, Some(12));
    }

    #[test]
    fn parse_location_tolerates_line_range_and_rejects_non_location() {
        assert_eq!(parse_location("X.agda:7,1-9,4").unwrap().1, Some(7));
        assert!(parse_location("  unused import ‘Foo’").is_none());
    }

    #[test]
    fn parse_findings_pairs_real_location_with_description() {
        // The `message` payload of REAL_UNUSED, with the \n unescaped.
        let msg = "/p/Main.agda:5,1-19\n  unused import ‘Helper’";
        let found = parse_findings(msg);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, PathBuf::from("/p/Main.agda"));
        assert_eq!(found[0].1, Some(5));
        assert_eq!(found[0].2, "unused import ‘Helper’");
    }
}
