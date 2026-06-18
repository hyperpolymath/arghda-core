//! The triage event stream.
//!
//! Every state transition appends one JSON object (one line) to
//! `<workspace>/.arghda/events.jsonl`. The log is append-only and is the
//! audit trail a downstream visual layer replays to reconstruct history.

use crate::timestamp::now_rfc3339;
use crate::workspace::State;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// What happened. Mirrors the transition table in `arghda-spec.adoc`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    Claim,
    Promote,
    Reject,
    Requeue,
    Invalidate,
}

/// A single transition record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub ts: String,
    pub kind: EventKind,
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<State>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<State>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Event {
    pub fn new(kind: EventKind, file: impl Into<String>) -> Self {
        Self {
            ts: now_rfc3339(),
            kind,
            file: file.into(),
            from: None,
            to: None,
            note: None,
        }
    }

    pub fn with_from(mut self, from: State) -> Self {
        self.from = Some(from);
        self
    }

    pub fn with_to(mut self, to: State) -> Self {
        self.to = Some(to);
        self
    }

    pub fn with_note(mut self, note: Option<String>) -> Self {
        self.note = note;
        self
    }
}

/// File name of the rolling event log, under `<workspace>/.arghda/`.
pub const EVENTS_FILE: &str = "events.jsonl";

fn events_path(ws_root: &Path) -> PathBuf {
    ws_root.join(".arghda").join(EVENTS_FILE)
}

/// Append one event as a JSON line. Creates `.arghda/` if absent.
pub fn append(ws_root: impl AsRef<Path>, ev: &Event) -> Result<()> {
    let ws_root = ws_root.as_ref();
    let meta = ws_root.join(".arghda");
    fs::create_dir_all(&meta).with_context(|| format!("creating {}", meta.display()))?;
    let path = events_path(ws_root);
    let mut line = serde_json::to_string(ev).context("serialising event")?;
    line.push('\n');
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    f.write_all(line.as_bytes())
        .with_context(|| format!("appending to {}", path.display()))?;
    Ok(())
}

/// Read the whole event log in order. Empty if the log does not exist yet.
pub fn read_all(ws_root: impl AsRef<Path>) -> Result<Vec<Event>> {
    let path = events_path(ws_root.as_ref());
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let contents =
        fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut out = Vec::new();
    for (i, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let ev: Event = serde_json::from_str(line)
            .with_context(|| format!("parsing {} line {}", path.display(), i + 1))?;
        out.push(ev);
    }
    Ok(out)
}
