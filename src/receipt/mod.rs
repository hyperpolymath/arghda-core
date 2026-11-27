// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Proof receipts: read them, and derive proven-ness from them.
//!
//! Design: `docs/design/derived-proven.adoc`. A module is proven exactly when
//! a *current* receipt for it records the digest of the module's current
//! input set, a lint-capped `proven` verdict, lint evidence, and no lint
//! hard block. Where a file sits (`proven/`) is never evidence.
//!
//! The receipt is the proof-receipt praxis deed of standards
//! `3-practice/PROOF-POLICY.adoc` §8.5.1. This reader also accepts the
//! fields the owner ruled should be proposed upstream (2026-11-25), which the
//! standards linter does not accept yet:
//!
//! * `:verdict` on a `(receipt …)`: the lint-capped verdict, as a kebab-case
//!   [`Verdict`] symbol (gap 3);
//! * `(lint :pack "…" (finding :rule "…" :severity hard-block|warn) …)` in a
//!   `(receipt …)`: the lint pack that ran and what it found (gap 3);
//! * `(theorem :name "…")` in a `(claim …)`: the module's headline theorems,
//!   listed and not graded (gap 2);
//! * `:profile workspace-receipt`: a receipt for an uncommitted worktree,
//!   where `:source-commit`, a receipt's `:commit` and `:run` may be absent
//!   and `(axioms …)` is optional (gap 1).
//!
//! Until those land in standards, arghda writes no receipts; this module is
//! tested against fixtures in that shape (the owner's fixture-first ruling).
//! It is a consumer, not the estate's gate: `deed_lint.js` stays normative.

pub mod sexp;

/// The one deed `:schema-version` this reader understands.
pub const SCHEMA_VERSION: &str = "1.0.0";

use crate::hash::sha256_hex;
use crate::prover::Verdict;
use crate::reason::cap_by_lints;
use anyhow::{bail, Context, Result};
use sexp::{Form, Sx};
use std::path::Path;

/// Which receipt profile a deed declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// Standards §8.5.1: committed sources, a CI run per receipt.
    ProofReceipt,
    /// Proposed (gap 1): an uncommitted worktree, keyed by its input digest.
    WorkspaceReceipt,
}

/// §8.4 lineage state of a receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptState {
    Current,
    Stale,
    Superseded,
    Regressed,
}

/// Severity of a lint finding recorded in a receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindingSeverity {
    HardBlock,
    Warn,
}

/// One lint finding recorded in a receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub rule: String,
    pub severity: FindingSeverity,
}

/// The `(lint …)` clause: which pack ran, and what it found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintEvidence {
    pub pack: String,
    pub findings: Vec<Finding>,
}

/// One `(receipt …)` record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub run: Option<String>,
    pub commit: Option<String>,
    pub checker: String,
    pub invocation: String,
    pub inputs_sha256: Option<String>,
    pub state: ReceiptState,
    pub reason: Option<String>,
    /// `(path, sha256)` in manifest order.
    pub manifest: Vec<(String, String)>,
    pub verdict: Option<Verdict>,
    pub lint: Option<LintEvidence>,
}

/// One `(leg …)`: a prover's receipts for a claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leg {
    pub prover: String,
    pub receipts: Vec<Receipt>,
}

/// One `(claim …)`: in arghda, one module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    pub id: String,
    pub theorems: Vec<String>,
    pub legs: Vec<Leg>,
}

/// A parsed receipt deed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptDeed {
    pub profile: Profile,
    pub repository: Option<String>,
    pub source_commit: Option<String>,
    pub claims: Vec<Claim>,
}

/// Why a derivation came out as it did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// No claim with this id.
    NoClaim,
    /// The claim has no leg for this prover.
    NoLeg,
    /// The leg has no current receipt.
    NoCurrentReceipt,
    /// The receipt's input digest is not the module's current one.
    Stale { recorded: String, actual: String },
    /// The receipt records no `:verdict` (a standards receipt today).
    NoVerdict,
    /// The receipt records no `(lint …)`, so a clean result cannot be told
    /// apart from a lint pack that never ran.
    NoLintEvidence,
    /// The receipt records these lint hard blocks.
    HardBlock(Vec<String>),
    /// The recorded verdict, capped by the recorded lint findings.
    Recorded,
}

/// The derived verdict for one module and prover, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Derivation {
    pub verdict: Verdict,
    pub why: Why,
}

impl Derivation {
    fn unknown(why: Why) -> Self {
        Self {
            verdict: Verdict::Unknown,
            why,
        }
    }
}

fn is_hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

fn parse_verdict(s: &str) -> Result<Verdict> {
    Ok(match s {
        "proven" => Verdict::Proven,
        "refuted" => Verdict::Refuted,
        "unknown" => Verdict::Unknown,
        "admitted" => Verdict::Admitted,
        "postulated" => Verdict::Postulated,
        "error" => Verdict::Error,
        "unavailable" => Verdict::Unavailable,
        _ => bail!("unknown :verdict `{s}`"),
    })
}

/// Refuse any field outside `allowed` (the field sets are closed, as in the
/// standards linter).
fn closed(form: &Form<'_>, allowed: &[&str], clauses: &[&str]) -> Result<()> {
    for (f, _) in &form.fields {
        if !allowed.contains(f) {
            bail!("({} …): unknown field :{f}", form.head);
        }
    }
    for c in &form.clauses {
        let h = Form::of(c)?.head;
        if !clauses.contains(&h) {
            bail!("({} …): unknown clause ({h} …)", form.head);
        }
    }
    Ok(())
}

/// §8.5.1 path rules: no newline or backslash, no empty, `.` or `..` part.
/// Also refused here: any other control character, such as `\r`, because
/// `sha256sum` escapes it in its line format and the two digest routes
/// would then disagree.
pub fn check_manifest_path(p: &str) -> Result<()> {
    if p.contains('\\') || p.chars().any(char::is_control) {
        bail!("manifest path {p:?} holds a control character or a backslash");
    }
    if p.split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        bail!("manifest path {p:?} has an empty, `.` or `..` part");
    }
    Ok(())
}

/// The §8.5.1 digest of a manifest already in order: SHA-256 over
/// `<sha256>  <path>\n` lines.
pub fn manifest_digest(files: &[(String, String)]) -> String {
    let mut m = String::new();
    for (path, sha) in files {
        m.push_str(sha);
        m.push_str("  ");
        m.push_str(path);
        m.push('\n');
    }
    sha256_hex(m.as_bytes())
}

/// Build the §8.5.1 manifest of `paths` (relative, `/`-separated) from the
/// worktree at `root`, sorted bytewise, and return it with its digest.
///
/// This reads the files on disk. For a committed file the bytes are the git
/// blob only when no checkout filter applies; a workspace receipt is about
/// the worktree, so that is the right source for it.
///
/// Symbolic links are refused, at every component of the path: git records
/// a link as its target *path*, while reading through it would hash the
/// target's *contents*, so the two routes could not agree, and a link could
/// reach outside `root`. A path must also be plain relative components (no
/// root, no drive prefix). The check runs before the read; a tree that
/// changes between the two is outside what this function can rule out.
pub fn worktree_manifest(root: &Path, paths: &[String]) -> Result<(Vec<(String, String)>, String)> {
    let mut sorted: Vec<&String> = paths.iter().collect();
    sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    sorted.dedup();
    let mut files = Vec::with_capacity(sorted.len());
    for p in sorted {
        check_manifest_path(p)?;
        let rel = Path::new(p.as_str());
        if rel
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            bail!("manifest path {p:?} is not a plain relative path");
        }
        let mut at = root.to_path_buf();
        for c in rel.components() {
            at.push(c);
            let meta = std::fs::symlink_metadata(&at).with_context(|| format!("reading {p}"))?;
            if meta.file_type().is_symlink() {
                bail!("manifest path {p:?} goes through a symbolic link");
            }
        }
        let bytes = std::fs::read(root.join(p)).with_context(|| format!("reading {p}"))?;
        files.push((p.clone(), sha256_hex(&bytes)));
    }
    if files.is_empty() {
        bail!("an input set must list at least one file");
    }
    let digest = manifest_digest(&files);
    Ok((files, digest))
}

fn read_manifest(sx: &Sx) -> Result<Vec<(String, String)>> {
    let form = Form::of(sx)?;
    closed(&form, &[], &["file"])?;
    read_files(&form)
}

/// The `(file :path … :sha256 …)` clauses of `form`, checked by the §8.5.1
/// rules: non-empty, strictly increasing bytewise, valid paths and digests.
fn read_files(form: &Form<'_>) -> Result<Vec<(String, String)>> {
    let mut files: Vec<(String, String)> = Vec::new();
    for f in form.clauses_named("file") {
        let ff = Form::of(f)?;
        closed(&ff, &["path", "sha256"], &[])?;
        let path = ff.str("path")?.context("(file …) needs :path")?;
        let sha = ff.str("sha256")?.context("(file …) needs :sha256")?;
        check_manifest_path(path)?;
        if !is_hex(sha, 64) {
            bail!("(file :path {path:?}): :sha256 must be 64 lowercase hex digits");
        }
        if let Some((prev, _)) = files.last() {
            if prev.as_bytes() >= path.as_bytes() {
                bail!("(inputs-manifest …): {path:?} is not after {prev:?} in bytewise order");
            }
        }
        files.push((path.to_string(), sha.to_string()));
    }
    if files.is_empty() {
        bail!("({} …) lists no file", form.head);
    }
    Ok(files)
}

/// Check every `(import :repository … :commit … :inputs-sha256 … (file …) …)`
/// clause of a receipt (§8.5.1, imported repositories): the files by the
/// manifest rules, the digest rebuilt from them, and one pin per repository.
fn check_imports(form: &Form<'_>) -> Result<()> {
    let mut seen: Vec<String> = Vec::new();
    for c in form.clauses_named("import") {
        let f = Form::of(c)?;
        closed(&f, &["repository", "commit", "inputs-sha256"], &["file"])?;
        let repo = f
            .str("repository")?
            .context("(import …) needs :repository")?;
        let (owner, name) = repo
            .split_once('/')
            .context("(import …): :repository must be OWNER/REPO")?;
        let ok = |s: &str| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
        };
        if !ok(owner) || !ok(name) {
            bail!("(import …): :repository must be OWNER/REPO");
        }
        let key = repo.to_ascii_lowercase();
        if seen.contains(&key) {
            bail!("(import …): {repo} is listed twice (one pin per repository)");
        }
        seen.push(key);
        match f.str("commit")? {
            Some(c) if is_hex(c, 40) => {}
            _ => bail!("(import {repo}): :commit must be the full 40-character commit"),
        }
        let d = f
            .str("inputs-sha256")?
            .filter(|d| is_hex(d, 64))
            .with_context(|| format!("(import {repo}): :inputs-sha256 must be 64 lowercase hex"))?;
        let files = read_files(&f)?;
        let got = manifest_digest(&files);
        if got != d {
            bail!(
                "(import {repo}): :inputs-sha256 is not the SHA-256 of its files (they give {got})"
            );
        }
    }
    Ok(())
}

/// The names a `:checker` may start with for a leg of this deed prover.
fn checker_names(prover: &str) -> &[&str] {
    match prover {
        "agda" => &["agda"],
        "rocq" | "coq" => &["rocq", "coq", "coqc"],
        "lean" => &["lean", "lean4"],
        "idris2" => &["idris2"],
        "isabelle" => &["isabelle"],
        "mizar" => &["mizar", "mizf"],
        "z3" => &["z3"],
        "cvc5" => &["cvc5"],
        _ => &[],
    }
}

/// A `:checker` must name the leg's own prover and a version
/// (`"agda 2.6.4.3"`), so a receipt from one tool cannot stand for another.
fn check_checker(prover: &str, checker: &str) -> Result<()> {
    let mut words = checker.split_whitespace();
    let tool = words.next().unwrap_or("");
    let version = words.next().unwrap_or("");
    let names = checker_names(prover);
    let named = if names.is_empty() {
        tool == prover
    } else {
        names.contains(&tool)
    };
    if !named || version.is_empty() {
        bail!(":checker {checker:?} must name the {prover} leg's prover and a version");
    }
    Ok(())
}

/// The deed prover symbol for an arghda backend name: receipts use the
/// standards vocabulary (`rocq`, `lean`; one `agda` leg for both Agda
/// profiles), arghda's backends use their tool names (`coq`, `lean4`,
/// `agda-cubical`).
pub fn deed_prover_for_backend(backend: &str) -> &str {
    match backend {
        "agda" | "agda-cubical" => "agda",
        "coq" => "rocq",
        "lean4" => "lean",
        other => other,
    }
}

fn read_lint(sx: &Sx) -> Result<LintEvidence> {
    let form = Form::of(sx)?;
    closed(&form, &["pack"], &["finding"])?;
    // The pack must be named: a blank pack would let a clause stand as lint
    // evidence without saying which pack ran. A name does not prove the run
    // (see the module docs on what a receipt can and cannot establish).
    let pack = form
        .str("pack")?
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .context("(lint …) needs a non-blank :pack")?
        .to_string();
    let mut findings = Vec::new();
    for f in form.clauses_named("finding") {
        let ff = Form::of(f)?;
        closed(&ff, &["rule", "severity"], &[])?;
        let rule = ff
            .str("rule")?
            .context("(finding …) needs :rule")?
            .to_string();
        let severity = match ff.sym("severity")?.context("(finding …) needs :severity")? {
            "hard-block" => FindingSeverity::HardBlock,
            "warn" => FindingSeverity::Warn,
            s => bail!("(finding …): unknown :severity `{s}`"),
        };
        findings.push(Finding { rule, severity });
    }
    Ok(LintEvidence { pack, findings })
}

fn read_receipt(sx: &Sx, profile: Profile, prover: &str) -> Result<Receipt> {
    let form = Form::of(sx)?;
    closed(
        &form,
        &[
            "run",
            "commit",
            "checker",
            "invocation",
            "inputs-sha256",
            "state",
            "reason",
            "image",
            "verdict",
        ],
        &["inputs-manifest", "axioms", "import", "lint"],
    )?;
    let committed = profile == Profile::ProofReceipt;
    let run = form.str("run")?.map(str::to_string);
    let commit = form.str("commit")?.map(str::to_string);
    if committed && run.is_none() {
        bail!("a proof-receipt (receipt …) needs :run");
    }
    match &commit {
        Some(c) if !is_hex(c, 40) => {
            bail!("a receipt :commit must be the full 40-character commit")
        }
        None if committed => bail!("a proof-receipt (receipt …) needs :commit"),
        _ => {}
    }
    let checker = form
        .str("checker")?
        .context("(receipt …) needs :checker")?
        .to_string();
    check_checker(prover, &checker)?;
    check_imports(&form)?;
    let invocation = form
        .str("invocation")?
        .context("(receipt …) needs :invocation")?
        .to_string();
    let state = match form.sym("state")?.context("(receipt …) needs :state")? {
        "current" => ReceiptState::Current,
        "stale" => ReceiptState::Stale,
        "superseded" => ReceiptState::Superseded,
        "regressed" => ReceiptState::Regressed,
        s => bail!("(receipt …): unknown :state `{s}`"),
    };
    let reason = form.str("reason")?.map(str::to_string);
    if state != ReceiptState::Current && reason.is_none() {
        bail!("a receipt that is not current needs :reason");
    }
    let inputs_sha256 = form.str("inputs-sha256")?.map(str::to_string);
    if let Some(d) = &inputs_sha256 {
        if !is_hex(d, 64) {
            bail!(":inputs-sha256 must be 64 lowercase hex digits");
        }
    }
    let manifests = form.clauses_named("inputs-manifest");
    if manifests.len() > 1 {
        bail!("a receipt has more than one (inputs-manifest …)");
    }
    let manifest = match manifests.first() {
        Some(m) => {
            let files = read_manifest(m)?;
            let Some(d) = &inputs_sha256 else {
                bail!("an (inputs-manifest …) needs :inputs-sha256");
            };
            let got = manifest_digest(&files);
            if &got != d {
                bail!(":inputs-sha256 is not the SHA-256 of its (inputs-manifest …) (the manifest gives {got})");
            }
            files
        }
        None => Vec::new(),
    };
    let axioms = form.clauses_named("axioms").len();
    if axioms > 1 {
        bail!("a receipt has more than one (axioms …)");
    }
    if state == ReceiptState::Current {
        if inputs_sha256.is_none() || manifest.is_empty() {
            bail!("a current receipt needs :inputs-sha256 and (inputs-manifest …)");
        }
        if committed && axioms == 0 {
            bail!("a current proof-receipt needs (axioms …)");
        }
    }
    let verdict = form.sym("verdict")?.map(parse_verdict).transpose()?;
    let lints = form.clauses_named("lint");
    if lints.len() > 1 {
        bail!("a receipt has more than one (lint …)");
    }
    let lint = lints.first().map(|l| read_lint(l)).transpose()?;
    Ok(Receipt {
        run,
        commit,
        checker,
        invocation,
        inputs_sha256,
        state,
        reason,
        manifest,
        verdict,
        lint,
    })
}

impl ReceiptDeed {
    /// Parse and check a receipt deed. Fails closed on anything outside the
    /// shapes described in the module docs.
    pub fn parse(src: &str) -> Result<Self> {
        let sx = sexp::parse(src)?;
        let top = Form::of(&sx)?;
        if top.head != "praxis-deed" {
            bail!("a receipt is a (praxis-deed …), not ({} …)", top.head);
        }
        closed(
            &top,
            &[
                "schema-version",
                "canonical-name",
                "beholding-chora",
                "profile",
                "repository",
                "source-commit",
            ],
            &["trusted-base", "claim", "echo"],
        )?;
        // deed.abnf: `:schema-version` STRING exactly once (Form::of refuses a
        // repeat). Only the version this reader knows is accepted, so a future,
        // possibly incompatible format is refused rather than misread.
        match top.str("schema-version")? {
            Some(SCHEMA_VERSION) => {}
            Some(v) => {
                bail!("unsupported :schema-version {v:?} (this reader knows {SCHEMA_VERSION:?})")
            }
            None => bail!("a receipt needs :schema-version"),
        }
        let profile = match top.sym("profile")?.context("a receipt needs :profile")? {
            "proof-receipt" => Profile::ProofReceipt,
            "workspace-receipt" => Profile::WorkspaceReceipt,
            p => bail!("not a receipt profile: `{p}`"),
        };
        let repository = top.str("repository")?.map(str::to_string);
        let source_commit = top.str("source-commit")?.map(str::to_string);
        if profile == Profile::ProofReceipt {
            if repository.is_none() {
                bail!("a proof-receipt needs :repository");
            }
            match &source_commit {
                Some(c) if is_hex(c, 40) => {}
                _ => bail!("a proof-receipt needs a 40-character :source-commit"),
            }
        }
        let mut claims: Vec<Claim> = Vec::new();
        for c in top.clauses_named("claim") {
            let cf = Form::of(c)?;
            closed(
                &cf,
                &["id", "pwg", "foundation-diverse"],
                &["leg", "control", "theorem"],
            )?;
            let id = cf.str("id")?.context("(claim …) needs :id")?.to_string();
            if claims.iter().any(|x| x.id == id) {
                bail!("duplicate claim id {id:?}");
            }
            let mut theorems = Vec::new();
            for t in cf.clauses_named("theorem") {
                let tf = Form::of(t)?;
                closed(&tf, &["name"], &[])?;
                theorems.push(
                    tf.str("name")?
                        .context("(theorem …) needs :name")?
                        .to_string(),
                );
            }
            let mut legs: Vec<Leg> = Vec::new();
            for l in cf.clauses_named("leg") {
                let lf = Form::of(l)?;
                closed(
                    &lf,
                    &["prover", "route", "token", "state", "reason"],
                    &["receipt"],
                )?;
                let prover = lf
                    .sym("prover")?
                    .context("(leg …) needs :prover")?
                    .to_string();
                if legs.iter().any(|x| x.prover == prover) {
                    bail!("claim {id:?} has two {prover} legs");
                }
                let receipts = lf
                    .clauses_named("receipt")
                    .into_iter()
                    .map(|r| read_receipt(r, profile, &prover))
                    .collect::<Result<Vec<_>>>()
                    .with_context(|| format!("claim {id:?}, {prover} leg"))?;
                if receipts
                    .iter()
                    .filter(|r| r.state == ReceiptState::Current)
                    .count()
                    > 1
                {
                    bail!("claim {id:?}: the {prover} leg has more than one current receipt");
                }
                legs.push(Leg { prover, receipts });
            }
            claims.push(Claim { id, theorems, legs });
        }
        Ok(Self {
            profile,
            repository,
            source_commit,
            claims,
        })
    }

    /// Derive the verdict of `claim` (a module) for `prover`, given the
    /// §8.5.1 digest of the module's input set as it is now.
    ///
    /// The result is `Proven` only when every condition of the receipt rule
    /// holds; any missing evidence gives `Unknown`, never a guess.
    /// [`ReceiptDeed::derive`] for an arghda backend name (`coq`, `lean4`,
    /// `agda-cubical`, …), mapped to the deed prover symbol first.
    pub fn derive_for_backend(
        &self,
        claim: &str,
        backend: &str,
        current_inputs_sha256: &str,
    ) -> Derivation {
        self.derive(
            claim,
            deed_prover_for_backend(backend),
            current_inputs_sha256,
        )
    }

    /// `prover` is the deed prover symbol of the leg (`agda`, `rocq`,
    /// `lean`, …); see [`deed_prover_for_backend`] and
    /// [`ReceiptDeed::derive_for_backend`] for arghda backend names.
    pub fn derive(&self, claim: &str, prover: &str, current_inputs_sha256: &str) -> Derivation {
        let Some(c) = self.claims.iter().find(|c| c.id == claim) else {
            return Derivation::unknown(Why::NoClaim);
        };
        let Some(leg) = c.legs.iter().find(|l| l.prover == prover) else {
            return Derivation::unknown(Why::NoLeg);
        };
        let Some(r) = leg
            .receipts
            .iter()
            .find(|r| r.state == ReceiptState::Current)
        else {
            return Derivation::unknown(Why::NoCurrentReceipt);
        };
        let recorded = r.inputs_sha256.clone().unwrap_or_default();
        if recorded != current_inputs_sha256 {
            return Derivation::unknown(Why::Stale {
                recorded,
                actual: current_inputs_sha256.to_string(),
            });
        }
        let Some(verdict) = r.verdict else {
            return Derivation::unknown(Why::NoVerdict);
        };
        let Some(lint) = &r.lint else {
            return Derivation::unknown(Why::NoLintEvidence);
        };
        let capped = cap_by_lints(verdict, lint.findings.iter().map(|f| f.rule.as_str()));
        let blocks: Vec<String> = lint
            .findings
            .iter()
            .filter(|f| f.severity == FindingSeverity::HardBlock)
            .map(|f| f.rule.clone())
            .collect();
        if !blocks.is_empty() {
            // A hard block is never evidence of anything better than unknown.
            let v = if matches!(
                capped,
                Verdict::Proven | Verdict::Admitted | Verdict::Postulated
            ) {
                Verdict::Unknown
            } else {
                capped
            };
            return Derivation {
                verdict: v,
                why: Why::HardBlock(blocks),
            };
        }
        Derivation {
            verdict: capped,
            why: Why::Recorded,
        }
    }
}

#[cfg(test)]
mod tests;
