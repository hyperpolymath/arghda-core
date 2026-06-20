// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Proven-state content hashing and staleness detection.

use arghda_core::{proven, State, Workspace};
use std::fs;

#[test]
fn promote_records_hash_and_stale_detects_a_later_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::init(tmp.path()).unwrap();
    fs::write(
        ws.state_dir(State::Working).join("Foo.agda"),
        "module Foo where\n",
    )
    .unwrap();

    ws.transition("Foo.agda", State::Working, State::Proven, None)
        .unwrap();

    // The promotion recorded a hash, and nothing is stale yet.
    let manifest = proven::load(tmp.path()).unwrap();
    assert!(manifest.entries.contains_key("Foo.agda"));
    assert_eq!(manifest.entries["Foo.agda"].sha256.len(), 64);
    assert!(ws.stale_proven().unwrap().is_empty());

    // Edit the proven file in place -> it is now stale.
    fs::write(
        ws.state_dir(State::Proven).join("Foo.agda"),
        "module Foo where\nx = Set\n",
    )
    .unwrap();
    let stale = ws.stale_proven().unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].file, "Foo.agda");
    assert_eq!(stale[0].reason, "content changed since promotion");
}

#[test]
fn invalidate_returns_to_inbox_and_drops_the_hash() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::init(tmp.path()).unwrap();
    fs::write(
        ws.state_dir(State::Working).join("Bar.agda"),
        "module Bar where\n",
    )
    .unwrap();
    ws.transition("Bar.agda", State::Working, State::Proven, None)
        .unwrap();

    // proven -> inbox invalidation.
    ws.transition(
        "Bar.agda",
        State::Proven,
        State::Inbox,
        Some("upstream changed".into()),
    )
    .unwrap();

    assert_eq!(ws.state_of("Bar.agda"), Some(State::Inbox));
    let manifest = proven::load(tmp.path()).unwrap();
    assert!(!manifest.entries.contains_key("Bar.agda"));
}

#[test]
fn editing_a_transitive_import_makes_a_proven_file_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::init(tmp.path()).unwrap();

    // A source tree (separate from the triage dirs) holding the dependency.
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("Dep.agda"), "module Dep where\nx = Set\n").unwrap();

    // Point the workspace at that source tree for closure hashing.
    fs::write(
        tmp.path().join(".arghda").join("config.toml"),
        format!("[proven]\ninclude_root = \"{}\"\n", src.display()),
    )
    .unwrap();

    // A file that depends on Dep, promoted to proven.
    fs::write(
        ws.state_dir(State::Working).join("Main.agda"),
        "module Main where\nopen import Dep\n",
    )
    .unwrap();
    ws.transition("Main.agda", State::Working, State::Proven, None)
        .unwrap();

    // Promotion recorded a closure hash, and nothing is stale yet.
    let manifest = proven::load(tmp.path()).unwrap();
    assert!(manifest.entries["Main.agda"].closure_sha256.is_some());
    assert!(ws.stale_proven().unwrap().is_empty());

    // Edit the DEPENDENCY, not the proven file itself -> stale via closure.
    fs::write(src.join("Dep.agda"), "module Dep where\nx = Set\ny = Set\n").unwrap();
    let stale = ws.stale_proven().unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].file, "Main.agda");
    assert_eq!(
        stale[0].reason,
        "a transitive import changed since promotion"
    );
}

#[test]
fn proven_file_without_a_record_is_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::init(tmp.path()).unwrap();
    // Drop a file straight into proven/ without going through promote, so no
    // hash was recorded — the audit case of "how did this get here?".
    fs::write(
        ws.state_dir(State::Proven).join("Loose.agda"),
        "module Loose where\n",
    )
    .unwrap();
    let stale = ws.stale_proven().unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].reason, "no recorded hash");
}
