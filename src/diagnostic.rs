use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    HardBlock,
    Warn,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub rule: String,
    pub severity: Severity,
    pub file: PathBuf,
    pub message: String,
    pub line: Option<usize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LintReport {
    pub file: PathBuf,
    pub diagnostics: Vec<Diagnostic>,
}

impl LintReport {
    pub fn new(file: PathBuf) -> Self {
        Self { file, diagnostics: Vec::new() }
    }

    pub fn push(&mut self, d: Diagnostic) {
        self.diagnostics.push(d);
    }

    pub fn has_hard_block(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == Severity::HardBlock)
    }

    pub fn hard_blocks(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.severity == Severity::HardBlock)
    }

    pub fn warns(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.severity == Severity::Warn)
    }
}
