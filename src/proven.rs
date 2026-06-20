// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! The `proven`-state content-hash manifest (`.arghda/hashes.json`).
//!
//! When a file is promoted to `proven`, its content hash is recorded here.
//! [`crate::Workspace::stale_proven`] recomputes and compares, so a `proven`
//! file that was edited (or never recorded) after promotion is surfaced and
//! can be sent back to `inbox` — the `proven -> inbox` invalidation from
//! `docs/arghda-spec.adoc`.
//!
//! v1 hashes the file content only. Hashing a file *plus its transitive
//! imports* (the spec's full form) needs the workspace to know the source
//! tree's include root, which the flat triage layout does not yet carry;
//! that is a documented follow-on.

use crate::hash::sha256_hex;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const HASHES_FILE: &str = "hashes.json";

/// What was recorded when a file entered `proven`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProvenRecord {
    pub sha256: String,
    pub promoted_at: String,
}

/// The whole manifest: basename -> record.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProvenManifest {
    #[serde(default)]
    pub entries: BTreeMap<String, ProvenRecord>,
}

fn manifest_path(ws_root: &Path) -> PathBuf {
    ws_root.join(".arghda").join(HASHES_FILE)
}

/// Load the manifest, or an empty one if it does not exist yet.
pub fn load(ws_root: &Path) -> Result<ProvenManifest> {
    let path = manifest_path(ws_root);
    if !path.is_file() {
        return Ok(ProvenManifest::default());
    }
    let contents =
        fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&contents).with_context(|| format!("parsing {}", path.display()))
}

/// Write the manifest, creating `.arghda/` if needed.
pub fn save(ws_root: &Path, manifest: &ProvenManifest) -> Result<()> {
    let meta = ws_root.join(".arghda");
    fs::create_dir_all(&meta).with_context(|| format!("creating {}", meta.display()))?;
    let path = manifest_path(ws_root);
    let mut json = serde_json::to_string_pretty(manifest).context("serialising manifest")?;
    json.push('\n');
    fs::write(&path, json).with_context(|| format!("writing {}", path.display()))
}

/// SHA-256 of a file's bytes, lowercase hex.
pub fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(sha256_hex(&bytes))
}
