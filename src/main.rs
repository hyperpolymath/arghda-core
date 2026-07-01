// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

use anyhow::{Context, Result};
use arghda_core::lint::LintContext;
use arghda_core::{
    build_dag, build_reason, event, run_lints, unused, watcher, Agda, Backend, Idris2, LintRule,
    RuleConfig, State, Workspace,
};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Duration;
use walkdir::WalkDir;

/// A lint pack (boxed trait objects).
type RuleSet = Vec<Box<dyn LintRule>>;

/// Resolve a `--backend` name to a backend instance. Agda is the default and
/// v0.1 reference; Idris2 is the estate ABI language.
fn backend_for(name: &str) -> Result<Box<dyn Backend>> {
    match name {
        "agda" => Ok(Box::new(Agda)),
        "idris2" => Ok(Box::new(Idris2)),
        other => anyhow::bail!("unknown backend `{other}` (known: agda, idris2)"),
    }
}

#[derive(Parser)]
#[command(
    name = "arghda",
    version,
    about = "Proof-workspace manager for provers/solvers (Agda, Idris2)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create the four-state workspace layout at PATH.
    Init { path: PathBuf },
    /// Lint every `.agda` file under PATH without moving anything.
    Scan {
        /// Directory containing `.agda` files; treated as the include root.
        path: PathBuf,
        /// Root module for orphan detection; repeatable. Defaults to every
        /// `All.agda`/`Smoke.agda` discovered under PATH.
        #[arg(long)]
        entry: Vec<PathBuf>,
        /// Regex a name must match to count as a pinnable headline
        /// (`unpinned-headline` rule). Defaults to the spec pattern.
        #[arg(long)]
        headline_pattern: Option<String>,
        /// Path to an `.arghda/config.toml`. Defaults to
        /// `<PATH>/.arghda/config.toml` if present.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Prover/solver backend to use: `agda` (default) or `idris2`.
        #[arg(long, default_value = "agda")]
        backend: String,
        /// Also run the external `agda-unused` analyser and re-emit its
        /// findings as `unused-import` warnings (requires `agda-unused` on
        /// PATH; skipped with a note if absent).
        #[arg(long)]
        unused: bool,
        /// Emit the report as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
    /// Typecheck one file with the backend and lint it; report the verdict.
    Check {
        /// The source file to check.
        file: PathBuf,
        /// Include root (search path). Defaults to the file's directory.
        #[arg(long)]
        include_root: Option<PathBuf>,
        /// Prover/solver backend to use: `agda` (default) or `idris2`.
        #[arg(long, default_value = "agda")]
        backend: String,
        /// Emit the report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Emit the dependency DAG (JSON) for the source tree at PATH.
    Dag {
        /// Directory containing `.agda` files; treated as the include root.
        path: PathBuf,
        /// Root module for orphan detection; repeatable. Defaults to every
        /// `All.agda`/`Smoke.agda` discovered under PATH.
        #[arg(long)]
        entry: Vec<PathBuf>,
        /// Regex a name must match to count as a pinnable headline
        /// (`unpinned-headline` rule). Defaults to the spec pattern.
        #[arg(long)]
        headline_pattern: Option<String>,
        /// Path to an `.arghda/config.toml`. Defaults to
        /// `<PATH>/.arghda/config.toml` if present.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Prover/solver backend to use: `agda` (default) or `idris2`.
        #[arg(long, default_value = "agda")]
        backend: String,
    },
    /// Emit the Flying-Logic reasoning graph (JSON) for the tree at PATH:
    /// the DAG plus a demote-only verdict propagation. Without `--check`,
    /// clean nodes are honestly `unknown`; postulate/pragma/escape-hatch
    /// caps still propagate downstream through import edges.
    Reason {
        /// Directory containing `.agda` files; treated as the include root.
        path: PathBuf,
        /// Root module for orphan detection; repeatable. Defaults to every
        /// `All.agda`/`Smoke.agda` discovered under PATH.
        #[arg(long)]
        entry: Vec<PathBuf>,
        /// Regex a name must match to count as a pinnable headline
        /// (`unpinned-headline` rule). Defaults to the spec pattern.
        #[arg(long)]
        headline_pattern: Option<String>,
        /// Path to an `.arghda/config.toml`. Defaults to
        /// `<PATH>/.arghda/config.toml` if present.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Prover/solver backend to use: `agda` (default) or `idris2`.
        #[arg(long, default_value = "agda")]
        backend: String,
        /// Run the backend on every node to populate REAL prover verdicts
        /// (honest exit codes). Slower — typechecks each module. Off by
        /// default, in which case clean nodes are `unknown`.
        #[arg(long)]
        check: bool,
    },
    /// Claim a file: inbox -> working.
    Claim { workspace: PathBuf, file: String },
    /// Promote a file: working -> proven.
    Promote { workspace: PathBuf, file: String },
    /// Reject a file: working -> rejected.
    Reject { workspace: PathBuf, file: String },
    /// Re-queue a file: rejected -> inbox.
    Requeue { workspace: PathBuf, file: String },
    /// Invalidate a proven file: proven -> inbox.
    Invalidate { workspace: PathBuf, file: String },
    /// Print the workspace event log.
    Events { workspace: PathBuf },
    /// List proven files whose content changed since promotion (stale).
    Stale {
        workspace: PathBuf,
        /// Move stale files back to inbox (proven -> inbox).
        #[arg(long)]
        invalidate: bool,
    },
    /// Watch `inbox/` and `working/` in a workspace; print events.
    Watch { workspace: PathBuf },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Init { path } => {
            let ws = Workspace::init(&path)?;
            println!("initialised workspace at {}", ws.root().display());
        }
        Cmd::Scan {
            path,
            entry,
            headline_pattern,
            config,
            backend,
            unused,
            json,
        } => scan(
            &path,
            &entry,
            headline_pattern.as_deref(),
            config.as_deref(),
            &backend,
            unused,
            json,
        )?,
        Cmd::Check {
            file,
            include_root,
            backend,
            json,
        } => check(&file, include_root.as_deref(), &backend, json)?,
        Cmd::Dag {
            path,
            entry,
            headline_pattern,
            config,
            backend,
        } => dag(
            &path,
            &entry,
            headline_pattern.as_deref(),
            config.as_deref(),
            &backend,
        )?,
        Cmd::Reason {
            path,
            entry,
            headline_pattern,
            config,
            backend,
            check,
        } => reason(
            &path,
            &entry,
            headline_pattern.as_deref(),
            config.as_deref(),
            &backend,
            check,
        )?,
        Cmd::Claim { workspace, file } => {
            transition(&workspace, &file, State::Inbox, State::Working)?
        }
        Cmd::Promote { workspace, file } => {
            transition(&workspace, &file, State::Working, State::Proven)?
        }
        Cmd::Reject { workspace, file } => {
            transition(&workspace, &file, State::Working, State::Rejected)?
        }
        Cmd::Requeue { workspace, file } => {
            transition(&workspace, &file, State::Rejected, State::Inbox)?
        }
        Cmd::Invalidate { workspace, file } => {
            transition(&workspace, &file, State::Proven, State::Inbox)?
        }
        Cmd::Events { workspace } => events(&workspace)?,
        Cmd::Stale {
            workspace,
            invalidate,
        } => stale(&workspace, invalidate)?,
        Cmd::Watch { workspace } => watch(&workspace)?,
    }
    Ok(())
}

fn scan(
    include_root: &Path,
    entry: &[PathBuf],
    headline_pattern: Option<&str>,
    config: Option<&Path>,
    backend_name: &str,
    unused: bool,
    json: bool,
) -> Result<()> {
    let backend = backend_for(backend_name)?;
    let (roots, rules, _cfg) = resolve_roots_and_rules(
        include_root,
        entry,
        headline_pattern,
        config,
        backend.as_ref(),
    )?;
    let ctx = LintContext {
        include_root,
        entry_modules: &roots,
    };
    let exts = backend.extensions();

    let mut reports = Vec::new();
    for entry in WalkDir::new(include_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let is_source = path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|e| exts.contains(&e));
        if !is_source {
            continue;
        }
        let report =
            run_lints(path, &ctx, &rules).with_context(|| format!("linting {}", path.display()))?;
        reports.push(report);
    }
    let files_scanned = reports.len();

    // Optional unused-code pass via the external `agda-unused` (per file,
    // local mode). Opt-in because it needs the external tool and re-checks
    // each file. Findings attach to that file's report.
    if unused {
        let mut available = true;
        let mut saw_error = false;
        for report in &mut reports {
            let check = unused::check_file(&report.file, include_root)?;
            if !check.available {
                available = false;
                break;
            }
            saw_error |= check.kind.as_deref() == Some("error");
            for d in check.diagnostics {
                report.push(d);
            }
        }
        if !available {
            eprintln!("note: `agda-unused` not found on PATH; skipping unused-import findings");
        } else if saw_error {
            eprintln!(
                "note: agda-unused could not analyse some files; unused-import findings may be incomplete"
            );
        }
    }

    let hard_blocks: usize = reports.iter().map(|r| r.hard_blocks().count()).sum();
    let warns: usize = reports.iter().map(|r| r.warns().count()).sum();

    if json {
        let payload = serde_json::json!({
            "version": "0.1",
            "include_root": include_root,
            "entry_modules": roots,
            "files_scanned": files_scanned,
            "hard_blocks": hard_blocks,
            "warns": warns,
            "reports": reports,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        for report in &reports {
            if report.diagnostics.is_empty() {
                continue;
            }
            println!("{}", report.file.display());
            for d in &report.diagnostics {
                println!("  [{}] {}: {}", sev_tag(d.severity), d.rule, d.message);
            }
        }
        println!(
            "\nscanned {} files; {} hard-block(s), {} warn(s)",
            reports.len(),
            hard_blocks,
            warns
        );
    }
    Ok(())
}

fn check(file: &Path, include_root: Option<&Path>, backend_name: &str, json: bool) -> Result<()> {
    if !file.is_file() {
        anyhow::bail!("file not found: {}", file.display());
    }
    let root_buf;
    let include_root = match include_root {
        Some(r) => r,
        None => {
            root_buf = file.parent().unwrap_or(Path::new(".")).to_path_buf();
            &root_buf
        }
    };

    // The orphan rule self-skips when the file is its own root, which is
    // exactly what a single-file check wants.
    let backend = backend_for(backend_name)?;
    let rules = backend.lint_rules(&RuleConfig::default())?;
    let roots = [file.to_path_buf()];
    let ctx = LintContext {
        include_root,
        entry_modules: &roots,
    };
    let report =
        run_lints(file, &ctx, &rules).with_context(|| format!("linting {}", file.display()))?;
    let outcome = backend.check_file(file, include_root)?;

    let verdict = if !outcome.available {
        "backend-unavailable"
    } else if outcome.ok && !report.has_hard_block() {
        "proven-eligible"
    } else {
        "rejected"
    };

    if json {
        let payload = serde_json::json!({
            "version": "0.1",
            "file": file,
            "backend": outcome,
            "lint": report,
            "verdict": verdict,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("{}", file.display());
        if outcome.available {
            println!(
                "  {}: exit {}, {}",
                backend.name(),
                outcome
                    .exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into()),
                if outcome.ok { "ok" } else { "FAILED" }
            );
        } else {
            println!(
                "  {}: not found on PATH (typecheck skipped)",
                backend.name()
            );
        }
        for d in &report.diagnostics {
            println!("  [{}] {}: {}", sev_tag(d.severity), d.rule, d.message);
        }
        println!("  verdict: {verdict}");
    }
    Ok(())
}

fn dag(
    include_root: &Path,
    entry: &[PathBuf],
    headline_pattern: Option<&str>,
    config: Option<&Path>,
    backend_name: &str,
) -> Result<()> {
    let backend = backend_for(backend_name)?;
    let (roots, rules, cfg) = resolve_roots_and_rules(
        include_root,
        entry,
        headline_pattern,
        config,
        backend.as_ref(),
    )?;
    let doc = build_dag(
        include_root,
        &roots,
        &rules,
        &cfg.headline_pattern,
        backend.as_ref(),
    )?;
    println!("{}", serde_json::to_string_pretty(&doc)?);
    Ok(())
}

fn reason(
    include_root: &Path,
    entry: &[PathBuf],
    headline_pattern: Option<&str>,
    config: Option<&Path>,
    backend_name: &str,
    do_check: bool,
) -> Result<()> {
    let backend = backend_for(backend_name)?;
    let (roots, rules, cfg) = resolve_roots_and_rules(
        include_root,
        entry,
        headline_pattern,
        config,
        backend.as_ref(),
    )?;
    let dag_doc = build_dag(
        include_root,
        &roots,
        &rules,
        &cfg.headline_pattern,
        backend.as_ref(),
    )?;

    // Real prover verdicts by module id. Empty unless `--check`, in which
    // case each node is typechecked for real (honest exit codes) — a green
    // node is only ever `proven` because the tool returned 0. Staleness
    // needs a workspace `proven/` state, so it stays empty here.
    let mut verdicts = std::collections::BTreeMap::new();
    let stale = std::collections::BTreeSet::new();
    if do_check {
        for node in &dag_doc.nodes {
            let file = include_root.join(&node.file);
            let outcome = backend.check_file(&file, include_root)?;
            if outcome.available {
                verdicts.insert(node.id.clone(), outcome.verdict);
            }
        }
    }

    let doc = build_reason(dag_doc, backend.as_ref(), &verdicts, &stale);
    println!("{}", serde_json::to_string_pretty(&doc)?);
    Ok(())
}

/// Build the lint `RuleConfig` with precedence default < `.arghda/config.toml`
/// < CLI flag. An explicit `--config` that does not exist is an error; the
/// default discovery location (`<include_root>/.arghda/config.toml`) is
/// silently optional.
fn resolve_config(
    include_root: &Path,
    config: Option<&Path>,
    headline_pattern: Option<&str>,
) -> Result<RuleConfig> {
    let mut cfg = match config {
        Some(p) => {
            if !p.is_file() {
                anyhow::bail!("config file not found: {}", p.display());
            }
            arghda_core::config::load_file(p)?
        }
        None => arghda_core::config::load_from_dir(include_root)?,
    };
    if let Some(p) = headline_pattern {
        cfg.headline_pattern = p.to_string();
    }
    Ok(cfg)
}

/// Resolve the root modules and the matching lint pack for `scan`/`dag`.
/// Explicit `--entry` values win; otherwise roots are auto-discovered
/// (`All.agda`/`Smoke.agda`). When no roots can be found, the
/// orphan-module rule is dropped (with a note) rather than flagging every
/// module as an orphan.
fn resolve_roots_and_rules(
    include_root: &Path,
    entry: &[PathBuf],
    headline_pattern: Option<&str>,
    config: Option<&Path>,
    backend: &dyn Backend,
) -> Result<(Vec<PathBuf>, RuleSet, RuleConfig)> {
    for e in entry {
        if !e.is_file() {
            anyhow::bail!("entry module not found: {}", e.display());
        }
    }
    let roots = if entry.is_empty() {
        backend.discover_roots(include_root)
    } else {
        entry.to_vec()
    };
    let cfg = resolve_config(include_root, config, headline_pattern)?;
    let rules: RuleSet = if roots.is_empty() {
        eprintln!(
            "note: no root modules (All.agda/Smoke.agda) found under {}; skipping orphan-module rule",
            include_root.display()
        );
        backend
            .lint_rules(&cfg)?
            .into_iter()
            .filter(|r| r.name() != "orphan-module")
            .collect()
    } else {
        backend.lint_rules(&cfg)?
    };
    Ok((roots, rules, cfg))
}

fn transition(workspace: &Path, file: &str, from: State, to: State) -> Result<()> {
    let ws = Workspace::open(workspace)?;
    ws.transition(file, from, to, None)?;
    println!("{file}: {} -> {}", from.dir_name(), to.dir_name());
    Ok(())
}

fn events(workspace: &Path) -> Result<()> {
    let events = event::read_all(workspace)?;
    if events.is_empty() {
        println!("(no events)");
        return Ok(());
    }
    for ev in &events {
        println!("{}", serde_json::to_string(ev)?);
    }
    Ok(())
}

fn stale(workspace: &Path, invalidate: bool) -> Result<()> {
    let ws = Workspace::open(workspace)?;
    let stale = ws.stale_proven()?;
    if stale.is_empty() {
        println!("(no stale proven files)");
        return Ok(());
    }
    for s in &stale {
        println!("stale: {} ({})", s.file, s.reason);
    }
    if invalidate {
        for s in &stale {
            ws.transition(
                &s.file,
                State::Proven,
                State::Inbox,
                Some(format!("auto-invalidated: {}", s.reason)),
            )?;
            println!("invalidated: {} (proven -> inbox)", s.file);
        }
    }
    Ok(())
}

fn watch(workspace_path: &Path) -> Result<()> {
    let ws = Workspace::open(workspace_path)?;
    let inbox = ws.state_dir(State::Inbox);
    let working = ws.state_dir(State::Working);

    let (_w_inbox, rx_inbox) = watcher::watch(&inbox, false)?;
    let (_w_working, rx_working) = watcher::watch(&working, false)?;

    println!("watching {} and {}", inbox.display(), working.display());
    println!("press ctrl-c to stop");

    loop {
        if let Ok(Ok(ev)) = rx_inbox.recv_timeout(Duration::from_millis(200)) {
            println!("inbox:   {:?} {:?}", ev.kind, ev.paths);
        }
        if let Ok(Ok(ev)) = rx_working.recv_timeout(Duration::from_millis(200)) {
            println!("working: {:?} {:?}", ev.kind, ev.paths);
        }
    }
}

fn sev_tag(sev: arghda_core::Severity) -> &'static str {
    match sev {
        arghda_core::Severity::HardBlock => "BLOCK",
        arghda_core::Severity::Warn => "warn",
    }
}
