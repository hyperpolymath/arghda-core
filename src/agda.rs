//! Shelling out to `agda` to typecheck a file.
//!
//! ArghDA never proves anything itself — it asks Agda. This module runs
//! the typechecker and captures the verdict. If `agda` is not on `PATH`
//! it degrades gracefully (`available: false`) rather than erroring, so
//! the rest of the engine still works in an Agda-less environment (CI
//! linting, DAG extraction, triage moves).

use anyhow::Result;
use serde::Serialize;
use std::path::Path;
use std::process::Command;

const TAIL_LINES: usize = 40;

/// Result of a typecheck attempt.
#[derive(Clone, Debug, Serialize)]
pub struct AgdaOutcome {
    /// Whether the `agda` binary was found and executed.
    pub available: bool,
    /// Process exit code, if the process ran.
    pub exit_code: Option<i32>,
    /// `true` iff agda exited 0.
    pub ok: bool,
    /// Last few lines of combined stdout+stderr (for surfacing errors).
    pub output_tail: String,
}

impl AgdaOutcome {
    fn unavailable() -> Self {
        Self {
            available: false,
            exit_code: None,
            ok: false,
            output_tail: String::new(),
        }
    }
}

/// Typecheck `file` with `include_root` on the search path
/// (`agda -i <include_root> <file>`).
pub fn check_file(file: &Path, include_root: &Path) -> Result<AgdaOutcome> {
    let output = Command::new("agda")
        .arg("-i")
        .arg(include_root)
        .arg(file)
        .output();

    match output {
        Ok(out) => {
            let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&out.stderr));
            Ok(AgdaOutcome {
                available: true,
                exit_code: out.status.code(),
                ok: out.status.success(),
                output_tail: tail(&combined, TAIL_LINES),
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AgdaOutcome::unavailable()),
        Err(e) => Err(e.into()),
    }
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_last_n_lines() {
        assert_eq!(tail("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(tail("only", 5), "only");
        assert_eq!(tail("", 3), "");
    }
}
