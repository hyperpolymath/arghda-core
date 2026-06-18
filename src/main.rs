use anyhow::{Context, Result};
use arghda_core::lint::LintContext;
use arghda_core::{
    build_dag, check_file, default_rules, event, graph, run_lints, watcher, LintRule, State,
    Workspace,
};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Duration;
use walkdir::WalkDir;

/// A lint pack (boxed trait objects).
type RuleSet = Vec<Box<dyn LintRule>>;

#[derive(Parser)]
#[command(
    name = "arghda",
    version,
    about = "Proof-workspace manager (Agda, v0.1)"
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
        /// Emit the report as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
    /// Typecheck one file with Agda and lint it; report the verdict.
    Check {
        /// The `.agda` file to check.
        file: PathBuf,
        /// Agda include root. Defaults to the file's directory.
        #[arg(long)]
        include_root: Option<PathBuf>,
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
        Cmd::Scan { path, entry, json } => scan(&path, &entry, json)?,
        Cmd::Check {
            file,
            include_root,
            json,
        } => check(&file, include_root.as_deref(), json)?,
        Cmd::Dag { path, entry } => dag(&path, &entry)?,
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
        Cmd::Watch { workspace } => watch(&workspace)?,
    }
    Ok(())
}

fn scan(include_root: &Path, entry: &[PathBuf], json: bool) -> Result<()> {
    let (roots, rules) = resolve_roots_and_rules(include_root, entry)?;
    let ctx = LintContext {
        include_root,
        entry_modules: &roots,
    };

    let mut reports = Vec::new();
    let mut hard_blocks = 0usize;
    let mut warns = 0usize;

    for entry in WalkDir::new(include_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("agda") {
            continue;
        }
        let report =
            run_lints(path, &ctx, &rules).with_context(|| format!("linting {}", path.display()))?;
        hard_blocks += report.hard_blocks().count();
        warns += report.warns().count();
        reports.push(report);
    }

    if json {
        let payload = serde_json::json!({
            "version": "0.1",
            "include_root": include_root,
            "entry_modules": roots,
            "files_scanned": reports.len(),
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

fn check(file: &Path, include_root: Option<&Path>, json: bool) -> Result<()> {
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
    let rules = default_rules();
    let roots = [file.to_path_buf()];
    let ctx = LintContext {
        include_root,
        entry_modules: &roots,
    };
    let report =
        run_lints(file, &ctx, &rules).with_context(|| format!("linting {}", file.display()))?;
    let agda = check_file(file, include_root)?;

    let verdict = if !agda.available {
        "agda-unavailable"
    } else if agda.ok && !report.has_hard_block() {
        "proven-eligible"
    } else {
        "rejected"
    };

    if json {
        let payload = serde_json::json!({
            "version": "0.1",
            "file": file,
            "agda": agda,
            "lint": report,
            "verdict": verdict,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("{}", file.display());
        if agda.available {
            println!(
                "  agda: exit {}, {}",
                agda.exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into()),
                if agda.ok { "ok" } else { "FAILED" }
            );
        } else {
            println!("  agda: not found on PATH (typecheck skipped)");
        }
        for d in &report.diagnostics {
            println!("  [{}] {}: {}", sev_tag(d.severity), d.rule, d.message);
        }
        println!("  verdict: {verdict}");
    }
    Ok(())
}

fn dag(include_root: &Path, entry: &[PathBuf]) -> Result<()> {
    let (roots, rules) = resolve_roots_and_rules(include_root, entry)?;
    let doc = build_dag(include_root, &roots, &rules)?;
    println!("{}", serde_json::to_string_pretty(&doc)?);
    Ok(())
}

/// Resolve the root modules and the matching lint pack for `scan`/`dag`.
/// Explicit `--entry` values win; otherwise roots are auto-discovered
/// (`All.agda`/`Smoke.agda`). When no roots can be found, the
/// orphan-module rule is dropped (with a note) rather than flagging every
/// module as an orphan.
fn resolve_roots_and_rules(
    include_root: &Path,
    entry: &[PathBuf],
) -> Result<(Vec<PathBuf>, RuleSet)> {
    for e in entry {
        if !e.is_file() {
            anyhow::bail!("entry module not found: {}", e.display());
        }
    }
    let roots = if entry.is_empty() {
        graph::discover_roots(include_root)
    } else {
        entry.to_vec()
    };
    let rules: RuleSet = if roots.is_empty() {
        eprintln!(
            "note: no root modules (All.agda/Smoke.agda) found under {}; skipping orphan-module rule",
            include_root.display()
        );
        default_rules()
            .into_iter()
            .filter(|r| r.name() != "orphan-module")
            .collect()
    } else {
        default_rules()
    };
    Ok((roots, rules))
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
