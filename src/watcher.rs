// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

use anyhow::Result;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc;

/// Minimal blocking watcher over a single directory.
/// Returns a receiver the caller can drain. Callers are responsible
/// for deciding what a given event means for triage state.
pub fn watch(
    path: impl AsRef<Path>,
    recursive: bool,
) -> Result<(
    RecommendedWatcher,
    mpsc::Receiver<notify::Result<notify::Event>>,
)> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx)?;
    let mode = if recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    watcher.watch(path.as_ref(), mode)?;
    Ok((watcher, rx))
}
