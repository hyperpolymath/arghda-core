// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! The Echidna dispatch seam.
//!
//! A backend's `check_file` can run one of two ways, both returning the SAME
//! [`Outcome`] contract:
//! * [`Dispatch::Local`] — shell out to the tool directly (the default, and
//!   the fully-real baseline).
//! * [`Dispatch::Echidna`] — route to the Echidna multi-prover orchestrator
//!   (Zig-API, port 8090) which runs the check elsewhere and returns the
//!   verdict.
//!
//! **Honest-stub discipline.** Echidna's orchestrator repo/API is unconfirmed
//! (it may not exist yet), and this seam explicitly *does not block on Echidna
//! existing*. Rather than invent an unverifiable HTTP contract and risk
//! fabricating a verdict, the Echidna route is an honest stub: it returns an
//! [`Outcome`] with `available: false` / [`Verdict::Unavailable`] and a clear
//! reason — never a made-up result. This mirrors the estate precedent
//! (cicd-squabbler's webhook server "exits EX_UNAVAILABLE rather than fake
//! OK"). Wiring the real orchestrator client (feature-gated) is the follow-on,
//! gated on Echidna's confirmed API.
//!
//! The value delivered now is the *seam*: one indirection point where a route
//! is chosen, with the `Outcome` contract preserved, so adding the real client
//! later touches only [`Dispatch::run`] — no backend or caller changes.

use crate::prover::{Backend, BackendKind, Outcome, Verdict};
use anyhow::{bail, Result};
use std::path::Path;

/// How a check is dispatched.
pub enum Dispatch {
    /// Shell out to the tool locally (default; fully real).
    Local,
    /// Route to the Echidna orchestrator at `base_url` (honest stub for now).
    Echidna { base_url: String },
}

impl Dispatch {
    /// The conventional Echidna orchestrator endpoint.
    pub const DEFAULT_ECHIDNA_URL: &'static str = "http://127.0.0.1:8090";

    /// Parse a `--dispatch` value: `local` (default), `echidna`, or
    /// `echidna=<url>`.
    pub fn parse(s: &str) -> Result<Dispatch> {
        if let Some((head, url)) = s.split_once('=') {
            if head == "echidna" {
                return Ok(Dispatch::Echidna {
                    base_url: url.to_string(),
                });
            }
        } else if s == "echidna" {
            return Ok(Dispatch::Echidna {
                base_url: Self::DEFAULT_ECHIDNA_URL.to_string(),
            });
        } else if s == "local" {
            return Ok(Dispatch::Local);
        }
        bail!("unknown --dispatch `{s}` (known: local, echidna, echidna=<url>)")
    }

    /// Run `backend`'s check via the chosen route. The `Outcome` contract is
    /// identical regardless of route.
    pub fn run(&self, backend: &dyn Backend, file: &Path, include_root: &Path) -> Result<Outcome> {
        match self {
            Dispatch::Local => backend.check_file(file, include_root),
            Dispatch::Echidna { base_url } => Ok(echidna_stub(backend.kind(), base_url)),
        }
    }
}

/// The honest Echidna stub: the seam is wired and returns the standard
/// `Outcome` shape, but reports `Unavailable` with a reason instead of
/// fabricating a verdict.
fn echidna_stub(kind: BackendKind, base_url: &str) -> Outcome {
    Outcome {
        available: false,
        exit_code: None,
        ok: false,
        output_tail: format!(
            "echidna dispatch seam is wired ({base_url}) but the orchestrator \
             client is not built in this configuration; no verdict fabricated"
        ),
        kind,
        verdict: Verdict::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prover::{Agda, Smt};

    #[test]
    fn parse_dispatch_values() {
        assert!(matches!(Dispatch::parse("local").unwrap(), Dispatch::Local));
        match Dispatch::parse("echidna").unwrap() {
            Dispatch::Echidna { base_url } => assert_eq!(base_url, Dispatch::DEFAULT_ECHIDNA_URL),
            _ => panic!("expected echidna"),
        }
        match Dispatch::parse("echidna=http://host:9000").unwrap() {
            Dispatch::Echidna { base_url } => assert_eq!(base_url, "http://host:9000"),
            _ => panic!("expected echidna with url"),
        }
        assert!(Dispatch::parse("nonsense").is_err());
    }

    #[test]
    fn echidna_route_is_an_honest_stub_never_fabricates() {
        // The seam preserves the backend's kind but reports Unavailable with
        // a reason — it must never claim Proven/Refuted it did not obtain.
        let d = Dispatch::parse("echidna").unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("q.smt2");
        std::fs::write(&f, "(check-sat)\n").unwrap();
        let out = d.run(&Smt::z3(), &f, tmp.path()).unwrap();
        assert_eq!(out.verdict, Verdict::Unavailable);
        assert!(!out.available);
        assert_eq!(out.kind, BackendKind::Solver, "kind is preserved");
        assert!(out.output_tail.contains("echidna"));
        assert!(out.output_tail.contains("no verdict fabricated"));
    }

    #[test]
    fn local_route_delegates_to_the_backend() {
        // Local dispatch is exactly backend.check_file — same honesty either
        // way (present ⇒ real verdict; absent ⇒ Unavailable).
        let d = Dispatch::Local;
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("X.agda");
        std::fs::write(&f, "module X where\n").unwrap();
        let via_dispatch = d.run(&Agda, &f, tmp.path()).unwrap();
        let direct = Agda.check_file(&f, tmp.path()).unwrap();
        assert_eq!(via_dispatch.available, direct.available);
        assert_eq!(via_dispatch.verdict, direct.verdict);
    }
}
