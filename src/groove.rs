// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>

//! Groove service-discovery manifest.
//!
//! Groove is the estate's HTTP-based service-discovery protocol: a capability
//! provider publishes a manifest at `/.well-known/groove` so consumers (e.g.
//! PanLL) can auto-detect it and wire it in without manual configuration.
//!
//! arghda-core is a CLI, not a server, so it *emits* the manifest (`arghda
//! groove`) as JSON; serving it over HTTP at `/.well-known/groove` is the
//! host's job (a web server, or the PanLL shell). Backend availability is
//! probed live, so the manifest reflects what this machine can actually run —
//! honest discovery, not a static advertisement.
//!
//! This is arghda's v0.1 manifest shape (`groove: "0.1"`); it announces the
//! two frozen JSON schemas the visual layer consumes (`dag/0.1`,
//! `reason/0.1`), the CLI commands, and the probed backends.

use crate::prover::Probe;
use serde::Serialize;

/// The JSON-contract schema versions the visual layer (arghda-studio) and
/// PanLL consume. Frozen — bump only on a breaking change.
#[derive(Clone, Debug, Serialize)]
pub struct Schemas {
    pub dag: &'static str,
    pub reason: &'static str,
}

/// What arghda announces it can do.
#[derive(Clone, Debug, Serialize)]
pub struct Capabilities {
    /// Live probe of each known backend (id, kind, runnable, detail).
    pub backends: Vec<Probe>,
    /// The CLI verbs a consumer can drive.
    pub commands: Vec<&'static str>,
    /// The frozen output-schema versions.
    pub schemas: Schemas,
}

/// The `/.well-known/groove` manifest for arghda.
#[derive(Clone, Debug, Serialize)]
pub struct GrooveManifest {
    /// Groove manifest-schema version.
    pub groove: &'static str,
    pub service: &'static str,
    pub description: &'static str,
    pub capabilities: Capabilities,
}

/// Build the manifest from live backend probes.
pub fn manifest(backends: Vec<Probe>) -> GrooveManifest {
    GrooveManifest {
        groove: "0.1",
        service: "arghda",
        description: "proof-workspace manager for provers and solvers",
        capabilities: Capabilities {
            backends,
            commands: vec!["scan", "check", "dag", "reason", "doctor", "groove"],
            schemas: Schemas {
                dag: "0.1",
                reason: "0.1",
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prover::BackendKind;

    fn probe(name: &str) -> Probe {
        Probe {
            backend: name.to_string(),
            kind: BackendKind::Assistant,
            runnable: true,
            detail: "v1".to_string(),
        }
    }

    #[test]
    fn manifest_announces_frozen_schemas_and_backends() {
        let m = manifest(vec![probe("agda"), probe("lean4")]);
        assert_eq!(m.groove, "0.1");
        assert_eq!(m.service, "arghda");
        assert_eq!(m.capabilities.schemas.dag, "0.1");
        assert_eq!(m.capabilities.schemas.reason, "0.1");
        assert_eq!(m.capabilities.backends.len(), 2);
        assert!(m.capabilities.commands.contains(&"reason"));
        // It serialises to the expected discovery shape.
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["groove"], "0.1");
        assert_eq!(v["capabilities"]["schemas"]["reason"], "0.1");
        assert_eq!(v["capabilities"]["backends"][0]["backend"], "agda");
    }
}
