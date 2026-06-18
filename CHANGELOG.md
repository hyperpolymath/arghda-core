<!-- SPDX-License-Identifier: MPL-2.0 -->
# Changelog

All notable changes to arghda-core are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and this project adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `dag` command emitting the dependency-DAG JSON contract (nodes with
  lint-derived status, import edges, and a `blocked` list covering
  self-blocks and prerequisite-blocks).
- `check` command: Agda typecheck fused with the lint report
  (`proven-eligible` / `rejected` / `agda-unavailable`); degrades gracefully
  when `agda` is absent.
- Workspace state machine — `claim` / `promote` / `reject` / `requeue` /
  `invalidate` as validated file moves, each logged to
  `.arghda/events.jsonl`; plus `events` to replay the log.
- First-class import graph (`graph` module) with multi-root reachability:
  orphan detection is the union of auto-discovered `All.agda` / `Smoke.agda`
  CI roots (or `--entry`, repeatable).
- Lint rules: `unjustified-postulate` (hard-block), `escape-hatch` (warn:
  `TERMINATING`-family pragmas + `believe_me` / `primTrustMe`), `tab-mix`
  (warn).
- RSR scaffolding: `.machine_readable/6a2/` artefacts, `0-AI-MANIFEST.a2ml`,
  `Justfile`, `.well-known/`, and community-health files.
- Content-hash invalidation of `proven`: promotion records a SHA-256 of the
  file in `.arghda/hashes.json`; the `stale` command reports proven files
  edited since promotion, and `stale --invalidate` moves them back to inbox
  (the `proven -> inbox` invalidation). Dependency-free SHA-256, pinned
  against the NIST test vectors.

### Notes

- Verified against Agda 2.6.3 and dogfooded on the echo-types corpus
  (193 modules, 903 import edges; the known `VarianceGate.agda` orphan and
  the out-of-cone files are surfaced correctly).

## [0.1.0] — 2026-05-30

- Initial extraction from echo-types: workspace scaffold, filesystem
  watcher, `missing-safe-pragma` + `orphan-module` lints, and the `scan` CLI.
