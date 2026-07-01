// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

use anyhow::{Context, Result};
use arghda_core::lint::LintContext;
use arghda_core::{
    build_dag, build_reason, event, groove_manifest, run_lints, unused, watcher, Agda, AgdaCubical,
    Backend, BackendKind, Coq, Dispatch, Idris2, Lean, LintRule, Probe, RuleConfig, Smt, State,
    Verdict, Workspace,
};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Duration;
use walkdir::WalkDir;

/// A lint pack (boxed trait objects).
type RuleSet = Vec<Box<dyn LintRule>>;

/// Every backend `--backend` accepts and `doctor` probes.
const KNOWN_BACKENDS: &[&str] = &[
    "agda",
    "agda-cubical",
    "idris2",
    "lean4",
    "coq",
    "z3",
    "cvc5",
];

/// Resolve a `--backend` name to a backend instance. Agda is the default and
/// v0.1 reference; Idris2 is the estate ABI language.
fn backend_for(name: &str) -> Result<Box<dyn Backend>> {
    match name {
        "agda" => Ok(Box::new(Agda)),
        "agda-cubical" => Ok(Box::new(AgdaCubical)),
        "idris2" => Ok(Box::new(Idris2)),
        "lean4" => Ok(Box::new(Lean)),
        "coq" => Ok(Box::new(Coq)),
        "z3" => Ok(Box::new(Smt::z3())),
        "cvc5" => Ok(Box::new(Smt::cvc5())),
        other => anyhow::bail!(
            "unknown backend `{other}` (known: agda, agda-cubical, idris2, lean4, coq, z3, cvc5)"
        ),
    }
}

#[derive(Parser)]
#[command(
    name = "arghda",
    version,
    about = "Proof-workspace manager for provers/solvers (Agda, Cubical, Idris2, Lean4, Coq, Z3, CVC5)"
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
        /// Backend: `agda` (default), `agda-cubical`, `idris2`, `lean4`,
        /// `coq`, `z3`, `cvc5`.
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
        /// Backend: `agda` (default), `agda-cubical`, `idris2`, `lean4`,
        /// `coq`, `z3`, `cvc5`.
        #[arg(long, default_value = "agda")]
        backend: String,
        /// Where the check runs: `local` (default) or `echidna[=<url>]`
        /// (route to the Echidna orchestrator).
        #[arg(long, default_value = "local")]
        dispatch: String,
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
        /// Backend: `agda` (default), `agda-cubical`, `idris2`, `lean4`,
        /// `coq`, `z3`, `cvc5`.
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
        /// Backend: `agda` (default), `agda-cubical`, `idris2`, `lean4`,
        /// `coq`, `z3`, `cvc5`.
        #[arg(long, default_value = "agda")]
        backend: String,
        /// Run the backend on every node to populate REAL prover verdicts
        /// (honest exit codes). Slower — typechecks each module. Off by
        /// default, in which case clean nodes are `unknown`.
        #[arg(long)]
        check: bool,
        /// With `--check`, where each check runs: `local` (default) or
        /// `echidna[=<url>]`.
        #[arg(long, default_value = "local")]
        dispatch: String,
        /// A four-state workspace whose `proven/` state supplies verdicts
        /// WITHOUT re-checking: a node whose file is proven (and fresh) →
        /// `proven`; proven-but-stale (content/closure hash changed) →
        /// demoted to `unknown`. Matched by file basename.
        #[arg(long)]
        workspace: Option<PathBuf>,
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
    /// Probe which prover/solver backends are actually runnable here.
    Doctor {
        /// Emit the probe results as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Emit the Groove service-discovery manifest (`/.well-known/groove`).
    /// Announces arghda's capabilities (probed backends, CLI commands, and
    /// the frozen `dag/0.1` + `reason/0.1` schemas) for PanLL discovery.
    Groove {
        /// Write the manifest to a file instead of stdout (e.g.
        /// `<site>/.well-known/groove`).
        #[arg(long)]
        output: Option<PathBuf>,
    },
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
            dispatch,
            json,
        } => check(&file, include_root.as_deref(), &backend, &dispatch, json)?,
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
            dispatch,
            workspace,
        } => reason(
            &path,
            &entry,
            headline_pattern.as_deref(),
            config.as_deref(),
            &backend,
            check,
            &dispatch,
            workspace.as_deref(),
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
        Cmd::Doctor { json } => doctor(json)?,
        Cmd::Groove { output } => groove(output.as_deref())?,
    }
    Ok(())
}

/// Probe every known backend for runnability (shared by `doctor`/`groove`).
fn probe_all() -> Result<Vec<Probe>> {
    KNOWN_BACKENDS
        .iter()
        .map(|name| backend_for(name).map(|b| b.probe()))
        .collect()
}

/// Emit the Groove service-discovery manifest (stdout, or `--output` file).
fn groove(output: Option<&Path>) -> Result<()> {
    let manifest = groove_manifest(probe_all()?);
    let json = serde_json::to_string_pretty(&manifest)?;
    match output {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(path, format!("{json}\n"))
                .with_context(|| format!("writing {}", path.display()))?;
            println!("wrote Groove manifest to {}", path.display());
        }
        None => println!("{json}"),
    }
    Ok(())
}

/// Probe every known backend and report which are actually runnable.
fn doctor(json: bool) -> Result<()> {
    let probes = probe_all()?;

    if json {
        let payload = serde_json::json!({ "version": "0.1", "backends": probes });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("arghda doctor — backend availability");
        for p in &probes {
            let mark = if p.runnable { "OK" } else { "--" };
            let kind = match p.kind {
                BackendKind::Assistant => "assistant",
                BackendKind::Solver => "solver",
            };
            println!("  [{mark}] {:<13} {:<10} {}", p.backend, kind, p.detail);
        }
        let runnable = probes.iter().filter(|p| p.runnable).count();
        println!("\n{}/{} backends runnable", runnable, probes.len());
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

fn check(
    file: &Path,
    include_root: Option<&Path>,
    backend_name: &str,
    dispatch: &str,
    json: bool,
) -> Result<()> {
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
    let outcome = Dispatch::parse(dispatch)?.run(backend.as_ref(), file, include_root)?;

    // Honest verdict: a lint hard-block rejects; otherwise report the
    // backend's ACTUAL verdict word. Only `Proven` (with no hard block) is
    // "proven-eligible" — a clean-but-unaudited Lean file is `unknown`, an
    // SMT `sat` is `refuted`, never silently "rejected"/"ok".
    let verdict = if !outcome.available {
        "backend-unavailable"
    } else if report.has_hard_block() {
        "rejected"
    } else {
        match outcome.verdict {
            Verdict::Proven => "proven-eligible",
            Verdict::Refuted => "refuted",
            Verdict::Unknown => "unknown",
            Verdict::Admitted => "admitted",
            Verdict::Postulated => "postulated",
            Verdict::Error => "rejected",
            Verdict::Unavailable => "backend-unavailable",
        }
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
                "  {}: exit {}",
                backend.name(),
                outcome
                    .exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into()),
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

#[allow(clippy::too_many_arguments)]
fn reason(
    include_root: &Path,
    entry: &[PathBuf],
    headline_pattern: Option<&str>,
    config: Option<&Path>,
    backend_name: &str,
    do_check: bool,
    dispatch: &str,
    workspace: Option<&Path>,
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

    // Real prover verdicts by module id, plus the stale set.
    let mut verdicts = std::collections::BTreeMap::new();
    let mut stale = std::collections::BTreeSet::new();

    // Source 1 — a workspace's `proven/` state (no re-check). The pure
    // mapping lives in `reason::workspace_verdicts`; here we just read the
    // proven + stale basename sets off the workspace and feed them in.
    if let Some(ws_path) = workspace {
        let ws = Workspace::open(ws_path)?;
        let proven: std::collections::BTreeSet<String> = ws
            .list(State::Proven)?
            .iter()
            .filter_map(|p| p.file_name().and_then(|s| s.to_str()).map(String::from))
            .collect();
        let stale_names: std::collections::BTreeSet<String> =
            ws.stale_proven()?.into_iter().map(|e| e.file).collect();
        let (wv, ws_stale) = arghda_core::workspace_verdicts(&dag_doc, &proven, &stale_names);
        verdicts.extend(wv);
        stale.extend(ws_stale);
    }

    // Source 2 — `--check`: typecheck each node for real (honest exit codes).
    // A fresh check is authoritative, so it overrides workspace verdicts.
    if do_check {
        let route = Dispatch::parse(dispatch)?;
        for node in &dag_doc.nodes {
            let file = include_root.join(&node.file);
            let outcome = route.run(backend.as_ref(), &file, include_root)?;
            if outcome.available {
                verdicts.insert(node.id.clone(), outcome.verdict);
                stale.remove(&node.id); // a fresh real verdict is not stale
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
