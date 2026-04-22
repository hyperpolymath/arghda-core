use super::{LintContext, LintRule};
use crate::diagnostic::{Diagnostic, LintReport, Severity};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub struct OrphanModule;

impl LintRule for OrphanModule {
    fn name(&self) -> &'static str {
        "orphan-module"
    }

    fn run(&self, file: &Path, ctx: &LintContext<'_>, report: &mut LintReport) -> Result<()> {
        let Some(module) = module_name_of(file, ctx.include_root) else {
            return Ok(()); // file sits outside include_root; nothing to say
        };

        // The entry module itself is never an orphan of itself.
        if Some(module.as_str()) == module_name_of(ctx.entry_module, ctx.include_root).as_deref() {
            return Ok(());
        }

        let reachable = transitive_imports(ctx.entry_module, ctx.include_root)
            .with_context(|| {
                format!(
                    "computing transitive imports from {}",
                    ctx.entry_module.display()
                )
            })?;

        if !reachable.contains(&module) {
            report.push(Diagnostic {
                rule: self.name().to_string(),
                severity: Severity::HardBlock,
                file: file.to_path_buf(),
                message: format!(
                    "module `{}` is not reachable via imports from `{}`",
                    module,
                    ctx.entry_module.display()
                ),
                line: None,
            });
        }

        Ok(())
    }
}

/// Relative module path for `file` under `include_root`.
/// Returns `None` if `file` is outside the root or not a `.agda` source.
fn module_name_of(file: &Path, include_root: &Path) -> Option<String> {
    let rel = file.strip_prefix(include_root).ok()?;
    let stem = rel.with_extension("");
    let mut parts = Vec::new();
    for comp in stem.components() {
        let std::path::Component::Normal(s) = comp else {
            return None;
        };
        parts.push(s.to_str()?.to_string());
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("."))
}

fn module_to_path(module: &str, include_root: &Path) -> PathBuf {
    let mut p = include_root.to_path_buf();
    for part in module.split('.') {
        p.push(part);
    }
    p.set_extension("agda");
    p
}

fn transitive_imports(entry: &Path, include_root: &Path) -> Result<HashSet<String>> {
    let mut reachable: HashSet<String> = HashSet::new();
    let mut worklist: Vec<String> = Vec::new();

    if let Some(m) = module_name_of(entry, include_root) {
        reachable.insert(m.clone());
        worklist.push(m);
    } else {
        // Entry is outside the include root; seed with its imports directly.
        for imp in direct_imports(entry)? {
            if reachable.insert(imp.clone()) {
                worklist.push(imp);
            }
        }
    }

    while let Some(module) = worklist.pop() {
        let path = module_to_path(&module, include_root);
        // Missing files (stdlib, external deps) are silently skipped.
        if !path.is_file() {
            continue;
        }
        for imp in direct_imports(&path)? {
            if reachable.insert(imp.clone()) {
                worklist.push(imp);
            }
        }
    }

    Ok(reachable)
}

/// Extract module names appearing in `import ...` / `open import ...`
/// top-level forms of `file`. Tolerant: stops at the first token after
/// `import`; ignores `hiding`, `using`, `as`, `public` modifiers.
fn direct_imports(file: &Path) -> Result<Vec<String>> {
    let contents = fs::read_to_string(file)
        .with_context(|| format!("reading {}", file.display()))?;
    let mut out = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") {
            continue;
        }
        // Accept `import X`, `open import X`, `open import X as Y`, etc.
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        let idx = tokens.iter().position(|&t| t == "import");
        let Some(i) = idx else { continue };
        // Reject lines where `import` is inside a string literal or embedded
        // in a larger identifier; a cheap heuristic is to require that the
        // token before `import` is empty, `open`, or absent.
        if i > 0 {
            let prev = tokens[i - 1];
            if prev != "open" {
                continue;
            }
        }
        let Some(module) = tokens.get(i + 1) else { continue };
        // Strip trailing punctuation Agda never allows in module paths.
        let cleaned = module.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '_');
        if cleaned.is_empty() {
            continue;
        }
        out.push(cleaned.to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_roundtrip() {
        let root = Path::new("/r");
        let file = Path::new("/r/Ordinal/Closure.agda");
        let name = module_name_of(file, root).unwrap();
        assert_eq!(name, "Ordinal.Closure");
        let back = module_to_path(&name, root);
        assert_eq!(back, PathBuf::from("/r/Ordinal/Closure.agda"));
    }

    #[test]
    fn parses_open_import_with_modifiers() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            "module M where\n\
             open import Data.Nat using (ℕ)\n\
             open import Foo.Bar\n\
             import Baz as B\n\
             -- open import IgnoredComment\n",
        )
        .unwrap();
        let imports = direct_imports(tmp.path()).unwrap();
        assert!(imports.contains(&"Data.Nat".to_string()));
        assert!(imports.contains(&"Foo.Bar".to_string()));
        assert!(imports.contains(&"Baz".to_string()));
        assert!(!imports.iter().any(|i| i.contains("Ignored")));
    }
}
