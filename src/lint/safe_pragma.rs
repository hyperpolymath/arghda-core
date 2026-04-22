use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub struct SafePragma;

const HEAD_LINES_SCANNED: usize = 30;

impl LintRule for SafePragma {
    fn name(&self) -> &'static str {
        "missing-safe-pragma"
    }

    fn run(&self, file: &Path, _ctx: &LintContext<'_>, report: &mut LintReport) -> Result<()> {
        let contents = fs::read_to_string(file)
            .with_context(|| format!("reading {}", file.display()))?;

        let mut safe_seen = false;
        let mut without_k_seen = false;

        for line in contents.lines().take(HEAD_LINES_SCANNED) {
            if !line.trim_start().starts_with("{-#") {
                continue;
            }
            if line.contains("OPTIONS") {
                if line.contains("--safe") {
                    safe_seen = true;
                }
                if line.contains("--without-K") {
                    without_k_seen = true;
                }
            }
        }

        if !safe_seen || !without_k_seen {
            let missing = match (safe_seen, without_k_seen) {
                (false, false) => "--safe and --without-K",
                (false, true) => "--safe",
                (true, false) => "--without-K",
                _ => unreachable!(),
            };
            report.push(Diagnostic {
                rule: self.name().to_string(),
                severity: Severity::HardBlock,
                file: file.to_path_buf(),
                message: format!(
                    "missing {} pragma in first {} lines",
                    missing, HEAD_LINES_SCANNED
                ),
                line: None,
            });
        }

        Ok(())
    }
}
