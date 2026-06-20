// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

use crate::event::{self, Event, EventKind};
use crate::proven::{self, ProvenRecord};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const STATE_DIRS: &[&str] = &["inbox", "working", "proven", "rejected"];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Inbox,
    Working,
    Proven,
    Rejected,
}

impl State {
    pub fn dir_name(self) -> &'static str {
        match self {
            State::Inbox => "inbox",
            State::Working => "working",
            State::Proven => "proven",
            State::Rejected => "rejected",
        }
    }
}

/// A workspace is a directory with the four state subdirs.
/// Its source of truth is the filesystem: transitions are file moves.
#[derive(Clone, Debug)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Create the workspace layout at `root`, idempotently.
    pub fn init(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .with_context(|| format!("creating workspace root {}", root.display()))?;
        for dir in STATE_DIRS {
            let p = root.join(dir);
            fs::create_dir_all(&p)
                .with_context(|| format!("creating state dir {}", p.display()))?;
        }
        let meta = root.join(".arghda");
        fs::create_dir_all(&meta)
            .with_context(|| format!("creating meta dir {}", meta.display()))?;
        Ok(Self { root })
    }

    /// Open an existing workspace; fails if any state dir is missing.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        for dir in STATE_DIRS {
            let p = root.join(dir);
            if !p.is_dir() {
                anyhow::bail!("workspace missing state dir: {}", p.display());
            }
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state_dir(&self, state: State) -> PathBuf {
        self.root.join(state.dir_name())
    }

    /// List files currently in `state`.
    pub fn list(&self, state: State) -> Result<Vec<PathBuf>> {
        let dir = self.state_dir(state);
        let mut out = Vec::new();
        for entry in
            fs::read_dir(&dir).with_context(|| format!("reading state dir {}", dir.display()))?
        {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                out.push(entry.path());
            }
        }
        Ok(out)
    }

    /// Which state currently holds `file_name` (a basename), if any.
    pub fn state_of(&self, file_name: &str) -> Option<State> {
        [State::Inbox, State::Working, State::Proven, State::Rejected]
            .into_iter()
            .find(|&state| self.state_dir(state).join(file_name).is_file())
    }

    /// Move `file_name` from `from` to `to`, appending the transition to the
    /// event log. Rejects transitions not in the spec's state machine, and
    /// fails if the file is not actually in `from`. Returns the new path.
    pub fn transition(
        &self,
        file_name: &str,
        from: State,
        to: State,
        note: Option<String>,
    ) -> Result<PathBuf> {
        let Some(kind) = transition_kind(from, to) else {
            anyhow::bail!(
                "disallowed transition {} -> {}",
                from.dir_name(),
                to.dir_name()
            );
        };
        let src = self.state_dir(from).join(file_name);
        if !src.is_file() {
            anyhow::bail!("`{}` is not in `{}`", file_name, from.dir_name());
        }
        let dst = self.state_dir(to).join(file_name);
        fs::rename(&src, &dst)
            .with_context(|| format!("moving {} -> {}", src.display(), dst.display()))?;

        let ev = Event::new(kind, file_name)
            .with_from(from)
            .with_to(to)
            .with_note(note);

        // Maintain the proven content-hash manifest: record on entry to
        // `proven`, drop on exit. This is what lets `stale_proven` detect a
        // file edited after promotion.
        if to == State::Proven {
            let mut manifest = proven::load(&self.root)?;
            manifest.entries.insert(
                file_name.to_string(),
                ProvenRecord {
                    sha256: proven::hash_file(&dst)?,
                    promoted_at: ev.ts.clone(),
                },
            );
            proven::save(&self.root, &manifest)?;
        } else if from == State::Proven {
            let mut manifest = proven::load(&self.root)?;
            if manifest.entries.remove(file_name).is_some() {
                proven::save(&self.root, &manifest)?;
            }
        }

        event::append(&self.root, &ev)
            .with_context(|| format!("logging {:?} for {}", kind, file_name))?;
        Ok(dst)
    }

    /// Proven files whose current content no longer matches the hash recorded
    /// at promotion (or that were never recorded). These are the candidates
    /// for `proven -> inbox` invalidation.
    pub fn stale_proven(&self) -> Result<Vec<StaleEntry>> {
        let manifest = proven::load(&self.root)?;
        let mut out = Vec::new();
        for path in self.list(State::Proven)? {
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let current = proven::hash_file(&path)?;
            let reason = match manifest.entries.get(name) {
                None => Some("no recorded hash"),
                Some(rec) if rec.sha256 != current => Some("content changed since promotion"),
                Some(_) => None,
            };
            if let Some(reason) = reason {
                out.push(StaleEntry {
                    file: name.to_string(),
                    reason,
                });
            }
        }
        out.sort_by(|a, b| a.file.cmp(&b.file));
        Ok(out)
    }
}

/// A `proven` file flagged stale by [`Workspace::stale_proven`].
#[derive(Clone, Debug)]
pub struct StaleEntry {
    pub file: String,
    pub reason: &'static str,
}

/// The event kind for a transition, or `None` if the pair is not a legal
/// move in the spec's state machine.
pub fn transition_kind(from: State, to: State) -> Option<EventKind> {
    use State::*;
    match (from, to) {
        (Inbox, Working) => Some(EventKind::Claim),
        (Working, Proven) => Some(EventKind::Promote),
        (Working, Rejected) => Some(EventKind::Reject),
        (Rejected, Inbox) => Some(EventKind::Requeue),
        (Proven, Inbox) => Some(EventKind::Invalidate),
        _ => None,
    }
}
