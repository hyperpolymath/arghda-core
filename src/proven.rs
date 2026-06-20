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
//! [`hash_file`] hashes the file's own bytes. [`closure_hash`] additionally
//! folds in every transitive import that resolves inside a source-tree
//! include root — the spec's full form, so a promoted file goes stale when a
//! proof *under* it changes, not only when it is edited. The include root is
//! a workspace property (`[proven] include_root` in `.arghda/config.toml`);
//! when it is unset, only own-bytes hashing applies.

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
    /// SHA-256 of the file's own bytes.
    pub sha256: String,
    /// SHA-256 of the file *and its transitive-import closure*, recorded when
    /// a source-tree include root was known at promotion. `None` for files
    /// promoted without one (own-bytes checking only); absent in older
    /// manifests, which still load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closure_sha256: Option<String>,
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

/// SHA-256 over `file`'s own bytes plus the bytes of every transitive import
/// that resolves to a file inside `include_root`, in deterministic module
/// order. This is the spec's full `proven` invalidation form: a promoted file
/// is stale not only when it is edited, but when any proof it depends on is.
///
/// `file` may live outside `include_root` (the usual case — it has been moved
/// into the triage `proven/` dir); its imports are still resolved against the
/// source tree at `include_root`. Imports that do not resolve in-tree (stdlib
/// / external) are skipped, exactly as the import graph omits them.
pub fn closure_hash(file: &Path, include_root: &Path) -> Result<String> {
    let mut deps: BTreeMap<String, String> = BTreeMap::new();
    for module in crate::graph::transitive_imports(file, include_root)? {
        let path = crate::graph::module_to_path(&module, include_root);
        if path.is_file() {
            deps.insert(module, hash_file(&path)?);
        }
    }
    // Entry's own hash first, then sorted `module\0hash` lines: deterministic,
    // and sensitive to a dependency being added, removed, or edited.
    let mut buf = String::with_capacity(72 * (deps.len() + 1));
    buf.push_str(&hash_file(file)?);
    buf.push('\n');
    for (module, hash) in &deps {
        buf.push_str(module);
        buf.push('\0');
        buf.push_str(hash);
        buf.push('\n');
    }
    Ok(sha256_hex(buf.as_bytes()))
}
