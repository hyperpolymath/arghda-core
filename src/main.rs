use anyhow::{Context, Result};
use arghda_core::{default_rules, run_lints, watcher, Workspace};
use arghda_core::lint::LintContext;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Duration;
use walkdir::WalkDir;

#[derive(Parser)]
#[command(name = "arghda", version, about = "Proof-workspace manager (Agda, v0.1)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create the four-state workspace layout at PATH.
    Init {
        path: PathBuf,
    },
    /// Lint every `.agda` file under PATH without moving anything.
    Scan {
        /// Directory containing `.agda` files; treated as the include root.
        path: PathBuf,
        /// Entry module used for orphan detection. Defaults to
        /// `<path>/All.agda`.
        #[arg(long)]
        entry: Option<PathBuf>,
        /// Emit the report as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
    /// Watch `inbox/` and `working/` in a workspace; print events.
    Watch {
        workspace: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Init { path } => {
            let ws = Workspace::init(&path)?;
            println!("initialised workspace at {}", ws.root().display());
        }
        Cmd::Scan { path, entry, json } => scan(&path, entry.as_deref(), json)?,
        Cmd::Watch { workspace } => watch(&workspace)?,
    }
    Ok(())
}

fn scan(include_root: &Path, entry: Option<&Path>, json: bool) -> Result<()> {
    let entry_buf;
    let entry = match entry {
        Some(e) => e,
        None => {
            entry_buf = include_root.join("All.agda");
            &entry_buf
        }
    };
    if !entry.is_file() {
        anyhow::bail!("entry module not found: {}", entry.display());
    }

    let rules = default_rules();
    let ctx = LintContext { include_root, entry_module: entry };

    let mut reports = Vec::new();
    let mut hard_blocks = 0usize;
    let mut warns = 0usize;

    for entry in WalkDir::new(include_root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("agda") {
            continue;
        }
        let report = run_lints(path, &ctx, &rules)
            .with_context(|| format!("linting {}", path.display()))?;
        hard_blocks += report.hard_blocks().count();
        warns += report.warns().count();
        reports.push(report);
    }

    if json {
        let payload = serde_json::json!({
            "version": "0.1",
            "include_root": include_root,
            "entry_module": entry,
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
                let tag = match d.severity {
                    arghda_core::Severity::HardBlock => "BLOCK",
                    arghda_core::Severity::Warn => "warn",
                };
                println!("  [{}] {}: {}", tag, d.rule, d.message);
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

fn watch(workspace_path: &Path) -> Result<()> {
    let ws = Workspace::open(workspace_path)?;
    let inbox = ws.state_dir(arghda_core::State::Inbox);
    let working = ws.state_dir(arghda_core::State::Working);

    let (_w_inbox, rx_inbox) = watcher::watch(&inbox, false)?;
    let (_w_working, rx_working) = watcher::watch(&working, false)?;

    println!("watching {} and {}", inbox.display(), working.display());
    println!("press ctrl-c to stop");

    loop {
        if let Ok(ev) = rx_inbox.recv_timeout(Duration::from_millis(200)) {
            if let Ok(ev) = ev {
                println!("inbox:   {:?} {:?}", ev.kind, ev.paths);
            }
        }
        if let Ok(ev) = rx_working.recv_timeout(Duration::from_millis(200)) {
            if let Ok(ev) = ev {
                println!("working: {:?} {:?}", ev.kind, ev.paths);
            }
        }
    }
}
